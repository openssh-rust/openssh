use super::Error;

use std::fs::File;
use std::fs::OpenOptions;
use std::io;

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;

/// Open "/dev/null" with RW.
fn get_null_fd() -> Result<RawFd, Error> {
    let err_msg =
        "std::fs::OpenOptions returns error that is not construct using Error::last_os_error";
    static NULL_FD: OnceCell<Result<File, i32>> = OnceCell::new();
    let res = NULL_FD.get_or_init(|| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(|err| err.raw_os_error().expect(err_msg))
    });

    match res {
        Ok(f) => Ok(AsRawFd::as_raw_fd(f)),
        Err(err_code) => {
            let io_err = io::Error::from_raw_os_error(*err_code);
            Err(Error::ChildIo(io_err))
        }
    }
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
