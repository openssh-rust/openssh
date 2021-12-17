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
// Optimize size of `Option<PipeFd>` by manually specifing the range.
// Shamelessly taken from [`io-lifetimes::OwnedFd`](https://github.com/sunfishcode/io-lifetimes/blob/8669b5a9fc1d0604d1105f6e39c77fa633ac9c71/src/types.rs#L99).
#[cfg_attr(rustc_attrs, rustc_layout_scalar_valid_range_start(0))]
// libstd/os/raw/mod.rs me that every libstd-supported platform has a
// 32-bit c_int.
//
// Below is -2, in two's complement, but that only works out
// because c_int is 32 bits.
#[cfg_attr(rustc_attrs, rustc_layout_scalar_valid_range_end(0xFF_FF_FF_FE))]
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
    #[cfg(feature = "native-mux")]
    pub(crate) fn try_clone(&self) -> Result<Self, Error> {
        // safety: self.0 is guaranteed to contain a valid fd.
        unsafe { dup(self.0) }
    }

    #[cfg(feature = "native-mux")]
    pub(crate) fn get_access_mode(&self) -> Result<AccessMode, Error> {
        // safety: self.0 is guaranteed to contain a valid fd.
        let res = unsafe { libc::fcntl(self.0, libc::F_GETFL) };

        if res == -1 {
            Err(Error::ChildIo(io::Error::last_os_error()))
        } else {
            Ok(AccessMode(res & libc::O_ACCMODE))
        }
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

#[cfg(feature = "native-mux")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct AccessMode(libc::c_int);

#[cfg(feature = "native-mux")]
impl AccessMode {
    /// Return true if the fd can only be read.
    pub(crate) const fn is_rdonly(&self) -> bool {
        self.0 == libc::O_RDONLY
    }

    /// Return true if the fd can only be write.
    pub(crate) const fn is_wronly(&self) -> bool {
        self.0 == libc::O_WRONLY
    }

    /// Return true if the fd is readable and writeable.
    pub(crate) const fn is_rdwr(&self) -> bool {
        self.0 == libc::O_RDWR
    }

    /// Return true if the fd can be read.
    pub(crate) const fn is_readable(&self) -> bool {
        self.is_rdonly() || self.is_rdwr()
    }

    /// Return true if the fd can be write.
    pub(crate) const fn is_writeable(&self) -> bool {
        self.is_wronly() || self.is_rdwr()
    }
}
