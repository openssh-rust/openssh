use super::Error;

use std::fs::{File, OpenOptions};

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;

/// Open "/dev/null" with RW.
fn get_null_fd() -> Result<RawFd, Error> {
    static NULL_FD: OnceCell<File> = OnceCell::new();
    let res = NULL_FD.get_or_try_init(|| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(Error::ChildIo)
    });

    res.map(AsRawFd::as_raw_fd)
}

pub(crate) fn into_fd<T: IntoRawFd>(val: T) -> File {
    unsafe { File::from_raw_fd(val.into_raw_fd()) }
}

pub(crate) fn as_raw_fd(fd: &Option<File>) -> Result<RawFd, Error> {
    match fd {
        Some(fd) => Ok(AsRawFd::as_raw_fd(fd)),
        None => get_null_fd(),
    }
}
