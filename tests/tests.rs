use std::{
    fs::File,
    io::{self, Read},
    os::unix::{
        io::{FromRawFd, RawFd},
        process::CommandExt,
    },
    process::Command,
};

use closefds::close_fds_on_exec;

fn pipe() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((fds[0], fds[1]))
}

#[test]
fn run_test() {
    let (r1, w1) = pipe().unwrap();

    let close_func = close_fds_on_exec(vec![0, 1, 2, w1]).unwrap();

    let (r2, w2) = pipe().unwrap();

    let mut cmd = Command::new("target/debug/test_prog");
    cmd.arg(format!("{}", w1));
    cmd.arg(format!("{}", r1));
    cmd.arg(format!("{}", w2));
    cmd.arg(format!("{}", r2));
    cmd.before_exec(close_func);
    let mut spawn = cmd.spawn().unwrap();

    unsafe {
        assert_eq!(libc::close(w1), 0);
        assert_eq!(libc::close(r2), 0);
        assert_eq!(libc::close(w2), 0);
    }

    let mut buf = vec![];
    let mut f = unsafe { File::from_raw_fd(r1) };
    f.read_to_end(&mut buf).unwrap();

    assert_eq!(buf.as_slice(), "x".as_bytes());

    let status = spawn.wait().unwrap();

    assert!(status.success());
}
