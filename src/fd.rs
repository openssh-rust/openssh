use std::fs::File;

#[cfg(feature = "mux_client")]
use std::fs::OpenOptions;
#[cfg(feature = "mux_client")]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

#[cfg(feature = "mux_client")]
use once_cell::sync::OnceCell;

/// Open "/dev/null" with RW.
#[cfg(feature = "mux_client")]
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

pub(crate) type Fd = File;

#[cfg(feature = "mux_client")]
pub(crate) fn into_fd<T: IntoRawFd>(val: T) -> Fd {
    unsafe { Fd::from_raw_fd(val.into_raw_fd()) }
}

#[cfg(feature = "mux_client")]
pub(crate) fn as_raw_fd(fd: &Option<Fd>) -> RawFd {
    match fd {
        Some(fd) => AsRawFd::as_raw_fd(fd),
        None => get_null_fd(),
    }
}
