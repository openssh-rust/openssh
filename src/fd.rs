use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use nix::unistd;
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

#[derive(Debug)]
pub(crate) struct Fd(RawFd);
impl Fd {
    pub(crate) fn into_raw_fd(self) -> RawFd {
        self.0
    }
}
impl Drop for Fd {
    fn drop(&mut self) {
        unistd::close(self.0).unwrap();
    }
}
impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
impl<T: IntoRawFd> From<T> for Fd {
    fn from(val: T) -> Self {
        Self(IntoRawFd::into_raw_fd(val))
    }
}
impl FromRawFd for Fd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(fd)
    }
}

pub(crate) fn as_raw_fd(fd: &Option<Fd>) -> RawFd {
    match fd {
        Some(fd) => fd.0,
        None => get_null_fd(),
    }
}
