use core::mem::ManuallyDrop;

#[cfg(feature = "mux_client")]
use core::mem::replace;

use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use std::process;

use crate::fd::Fd;

#[derive(Debug)]
pub(crate) enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(Fd),
}

/// Similar to std::process::Stdio
#[derive(Debug)]
pub struct Stdio(pub(crate) StdioImpl);
impl Stdio {
    /// Create a pipe for child communication
    pub const fn piped() -> Self {
        Self(StdioImpl::Pipe)
    }

    /// Pass /dev/null for child communication
    pub const fn null() -> Self {
        Self(StdioImpl::Null)
    }

    /// Take the value and replace it with Stdio::null()
    #[cfg(feature = "mux_client")]
    pub(crate) fn take(&mut self) -> Self {
        replace(self, Stdio::null())
    }
}
impl<T: IntoRawFd> From<T> for Stdio {
    fn from(val: T) -> Self {
        Self(StdioImpl::Fd(val.into()))
    }
}
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(fd.into()))
    }
}
impl From<Stdio> for process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            StdioImpl::Null => process::Stdio::null(),
            StdioImpl::Pipe => process::Stdio::piped(),
            StdioImpl::Fd(fd) => unsafe {
                process::Stdio::from_raw_fd(ManuallyDrop::new(fd).as_raw_fd())
            },
        }
    }
}
