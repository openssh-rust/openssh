use std::fs::File;
use std::fs::OpenOptions;

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;

/// Open "/dev/null" with RW.
fn get_null_fd() -> RawFd {
    static NULL_FD: OnceCell<RawFd> = OnceCell::new();
    *NULL_FD.get_or_init(|| {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap();
        IntoRawFd::into_raw_fd(file)
    })
}

pub(crate) fn into_fd<T: IntoRawFd>(val: T) -> File {
    unsafe { File::from_raw_fd(val.into_raw_fd()) }
}

pub(crate) fn as_raw_fd(fd: &Option<File>) -> RawFd {
    match fd {
        Some(fd) => AsRawFd::as_raw_fd(fd),
        None => get_null_fd(),
    }
}
