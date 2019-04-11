use std::{
    env,
    fs::File,
    io::{self, Write},
    mem,
    os::unix::io::FromRawFd,
};

fn main() {
    let args: Vec<String> = env::args().into_iter().collect();

    let w1: libc::c_int = args[1].parse().unwrap();
    let r1: libc::c_int = args[2].parse().unwrap();
    let w2: libc::c_int = args[3].parse().unwrap();
    let r2: libc::c_int = args[4].parse().unwrap();

    let mut f = unsafe { File::from_raw_fd(w1) };
    f.write_all(b"x").unwrap();

    unsafe {
        assert_eq!(libc::close(r1), -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error().unwrap(),
            libc::EBADF
        );

        mem::forget(f);
        assert_eq!(libc::close(w1), 0);

        assert_eq!(libc::close(r2), -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error().unwrap(),
            libc::EBADF
        );
        assert_eq!(libc::close(w2), -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error().unwrap(),
            libc::EBADF
        );
    }
}
