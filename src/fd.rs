use super::Error;

use std::io;
use std::mem::ManuallyDrop;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub(crate) unsafe fn dup(raw_fd: RawFd) -> Result<Fd, Error> {
    let res = libc::dup(raw_fd);
    if res == -1 {
        Err(Error::ChildIo(io::Error::last_os_error()))
    } else {
        Ok(Fd(res))
    }
}

/// RAII wrapper for RawFd
#[derive(Debug)]
pub(crate) struct Fd(RawFd);

impl FromRawFd for Fd {
    unsafe fn from_raw_fd(raw_fd: RawFd) -> Self {
        Self(raw_fd)
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl IntoRawFd for Fd {
    fn into_raw_fd(self) -> RawFd {
        ManuallyDrop::new(self).0
    }
}

impl Fd {
    pub(crate) fn try_clone(&self) -> Result<Self, Error> {
        // safety: self.0 is guaranteed to contain a valid fd.
        unsafe { dup(self.0) }
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        // safety: self.0 is guaranteed to contain a valid fd.
        let res = unsafe { libc::close(self.0) };

        debug_assert!(
            res != -1,
            "Error when closing fd {}: {}",
            self.0,
            io::Error::last_os_error()
        );
    }
}
