use super::Error;

use io_lifetimes::OwnedFd;
use std::io;
use std::os::unix::io::{FromRawFd, RawFd};

pub(crate) unsafe fn dup(raw_fd: RawFd) -> Result<OwnedFd, Error> {
    let res = libc::dup(raw_fd);
    if res == -1 {
        Err(Error::ChildIo(io::Error::last_os_error()))
    } else {
        // safety: dup returns a valid fd on success.
        Ok(OwnedFd::from_raw_fd(res))
    }
}
