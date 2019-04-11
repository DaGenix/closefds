// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! closefds is a library that provides support for setting the `FD_CLOEXEC` flag on all open
//! file descriptors after `fork()` and before `exec()` on UNIX-like systems.
//!
//! Any file descriptors that aren't marked with this flag will stay open after `exec()`
//! which can cause resources to leak and can lead to deadlocks. Ideally, whenever a file
//! descriptor is created, it will be created with the `FD_CLOEXEC` flag already set.
//! However, this may not be possible in some circumstances - such as when using an
//! external library or a system call that does not support the `FD_CLOEXEC` flag, such as
//! `pipe()`.
//!
//! The function `close_fds_on_exec()` will create a closure that can be passed
//! as a `pre_exec()` function when spawning a child process via the `Command` interface
//! and will set the `FD_CLOEXEC` flag as appropriate on open file descriptors.

use std::{ffi::CStr, io, os::unix::io::RawFd, ptr};

#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
))]
const FD_DIR_NAME: &'static [u8; 8] = b"/dev/fd\0";

#[cfg(target_os = "linux")]
const FD_DIR_NAME: &'static [u8; 14] = b"/proc/self/fd\0";

struct OpenDir {
    dir: *mut libc::DIR,
}

// My best understanding is that functions that work with a libc::DIR
// do the appropriate locking to make it safe to work with from
// multiple threads.
unsafe impl Send for OpenDir {}
unsafe impl Sync for OpenDir {}

impl OpenDir {
    fn open(dir_path: &CStr) -> io::Result<OpenDir> {
        let dir = unsafe { libc::opendir(dir_path.as_ptr()) };
        if dir == ptr::null_mut() {
            return Err(io::Error::last_os_error());
        }
        Ok(OpenDir { dir })
    }
}

impl Drop for OpenDir {
    fn drop(&mut self) {
        // This will likely call free() - which is why the closure that
        // is created by close_fds_on_exec() should not be dropped by
        // the child process after fork().
        let _ = unsafe { libc::closedir(self.dir) };
    }
}

fn set_cloexec(fd: RawFd, set: bool) -> io::Result<()> {
    let mut fd_flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if fd_flags == -1 {
        return Err(io::Error::last_os_error());
    }

    let is_set = fd_flags & libc::FD_CLOEXEC != 0;

    if set == is_set {
        return Ok(());
    } else {
        if set {
            fd_flags |= libc::FD_CLOEXEC;
        } else {
            fd_flags &= !libc::FD_CLOEXEC;
        }
    }

    if unsafe { libc::fcntl(fd, libc::F_SETFD, fd_flags) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

unsafe fn pos_int_from_ascii(mut name: *const libc::c_char) -> io::Result<libc::c_int> {
    let mut num = 0;
    while *name >= '0' as i8 && *name <= '9' as i8 {
        num = num * 10 + (*name - '0' as i8) as libc::c_int;
        name = name.offset(1);
    }
    // If the last byte isn't a NULL, it means we found a
    // non-digit.
    if *name != 0 {
        errno::set_errno(errno::Errno(libc::ENOENT));
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "fd file name contained non-integer characters",
        ));
    }
    Ok(num)
}

struct CloseFdsOnExec {
    dir: OpenDir,
    keep_fds: Vec<RawFd>,
}

impl CloseFdsOnExec {
    pub fn new(mut keep_fds: Vec<RawFd>) -> io::Result<Self> {
        let dir = OpenDir::open(CStr::from_bytes_with_nul(FD_DIR_NAME).expect("Invalid Path"))?;
        keep_fds.sort_unstable();
        Ok(CloseFdsOnExec { dir, keep_fds })
    }

    pub fn before_exec(&mut self) -> io::Result<()> {
        unsafe {
            errno::set_errno(errno::Errno(0));
            libc::rewinddir(self.dir.dir);
            if errno::errno() != errno::Errno(0) {
                return Err(io::Error::last_os_error());
            }

            loop {
                errno::set_errno(errno::Errno(0));
                let dir_entry = libc::readdir(self.dir.dir);
                if dir_entry == ptr::null_mut() {
                    if errno::errno() != errno::Errno(0) {
                        return Err(io::Error::last_os_error());
                    } else {
                        break;
                    }
                }

                let f = pos_int_from_ascii((*dir_entry).d_name.as_ptr())?;
                let needs_cloexec = self.keep_fds.binary_search(&f).is_err();
                set_cloexec(f, needs_cloexec)?;
            }
        }
        Ok(())
    }
}

/// Create a closure that will set the `FD_CLOEXEC` flag on all open file descriptors when called.
/// This function should be called _before_ invoking `fork()` as it may allocate memory. The
/// returned closure should be called after `fork()` by the child process.
///
/// This function may fail to create the closure. Additionally, the closure may
/// also fail when called. However, in no case will we fall back to an implementation that does not
/// guarantee that all open file descriptors have been successfully processed (ie: We will not
/// look up the max number of open file descriptors and then attempt to close all file
/// descriptors up to that number as such an approach may fail to process some file descriptors
/// if the max number of open file descriptors changes).
///
/// `keep_fds` is a `Vec` of file descriptors to ensure that the `FD_CLOEXEC` flag is
/// _not_ set on. `FD_CLOEXEC` will be set on all other file descriptors.
///
/// # Current Implementation
///
/// The current implementation opens either the `/proc/self/fd/` directory (Linux) or `/dev/fd/`
/// directory (BSDs) in the parent process with `opendir()`. `readdir()` is used in the child
/// process to iterate over the entries in that directory and set the `FD_CLOEXEC` flag as
/// appropriate.
///
/// Notes:
///
/// * `readdir()` is not async-signal-safe according to any standard. However, the process
/// spawning code in both Python and Java work similarly, so `readdir()` seems
/// to be safe to call in practice after `fork()`.
///
/// * `/proc/self/fd/` or `/dev/fd/` directories _must_ be available.
///
/// * The returned closure needs to be dropped in the parent process in order to close
/// the opened directory. However, it must not be dropped in the child process as doing
/// so will call `free()` which may deadlock - all resources will instead be freed when
/// `exec()` occurs. (The standard library `CommandExt` interface does not drop closures
/// before `exec()`).
///
/// # Future Implementations
///
/// A future version of this library may change the implementation for all supported operating
/// systems or for specific operating systems. The likely reasons to do so would be to improve
/// performance, to remove calls to non-async-signal-safe functions, or to remove the dependency
/// on specific directories being available. However, any future implementation is guaranteed
/// to still process all open file descriptors.
///
/// # Example
///
/// The following example will spawn a child process while making sure that only STDIN, STDOUT, and
/// STDERR are inherited.
///
/// ```no_run
/// # use closefds::close_fds_on_exec;
/// # use std::process::Command;
/// # use std::os::unix::process::CommandExt;
/// # fn main() -> std::io::Result<()> {
/// # unsafe {
/// Command::new("path/to/program")
///     .pre_exec(close_fds_on_exec(vec![0, 1, 2])?)
///     .spawn()
///     .expect("Spawn Failed");
/// # }
/// # Ok(())
/// # }
/// ```
pub fn close_fds_on_exec(keep_fds: Vec<RawFd>) -> io::Result<impl FnMut() -> io::Result<()>> {
    let mut close_fds_on_exec = CloseFdsOnExec::new(keep_fds)?;

    let func = move || close_fds_on_exec.before_exec();

    Ok(func)
}

#[allow(dead_code)]
fn assert_traits() {
    fn check_traits<T: Send + Sync + 'static>(_: T) {}

    check_traits(close_fds_on_exec(vec![]));
    check_traits(CloseFdsOnExec::new(vec![]));
}
