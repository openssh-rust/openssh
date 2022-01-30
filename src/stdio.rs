use super::Error;

#[cfg(feature = "native-mux")]
use super::native_mux_impl;

use io_lifetimes::OwnedFd;
use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::process;

use std::convert::TryFrom;
use std::convert::TryInto;

pub(crate) unsafe fn dup(raw_fd: RawFd) -> Result<OwnedFd, Error> {
    let res = libc::dup(raw_fd);
    if res == -1 {
        Err(Error::ChildIo(io::Error::last_os_error()))
    } else {
        // safety: dup returns a valid fd on success.
        Ok(OwnedFd::from_raw_fd(res))
    }
}

#[derive(Debug)]
pub(crate) enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(OwnedFd),
    /// Inherit stdin/stdout/stderr
    Inherit,
}

/// Describes what to do with a standard I/O stream for a remote child process
/// when passed to the stdin, stdout, and stderr methods of Command.
#[derive(Debug)]
pub struct Stdio(pub(crate) StdioImpl);
impl Stdio {
    /// A new pipe should be arranged to connect the parent and remote child processes.
    pub const fn piped() -> Self {
        Self(StdioImpl::Pipe)
    }

    /// This stream will be ignored.
    /// This is the equivalent of attaching the stream to /dev/null.
    pub const fn null() -> Self {
        Self(StdioImpl::Null)
    }

    /// The child inherits from the corresponding parent descriptor.
    pub const fn inherit() -> Self {
        Self(StdioImpl::Inherit)
    }
}
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(OwnedFd::from_raw_fd(fd)))
    }
}
impl From<Stdio> for process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            StdioImpl::Null => process::Stdio::null(),
            StdioImpl::Pipe => process::Stdio::piped(),
            StdioImpl::Inherit => process::Stdio::inherit(),

            // safety: StdioImpl(fd) is only constructed from known-valid and
            // owned file descriptors by virtue of the safety requirement
            // for invoking from_raw_fd.
            StdioImpl::Fd(fd) => unsafe { process::Stdio::from_raw_fd(IntoRawFd::into_raw_fd(fd)) },
        }
    }
}

impl From<OwnedFd> for Stdio {
    fn from(fd: OwnedFd) -> Self {
        Self(StdioImpl::Fd(fd))
    }
}

macro_rules! impl_from_for_stdio {
    ($type:ty) => {
        impl From<$type> for Stdio {
            fn from(arg: $type) -> Self {
                let fd = arg.into_raw_fd();
                // safety: $type must have a valid into_raw_fd implementation
                // and must not be RawFd.
                unsafe { Self::from_raw_fd(fd) }
            }
        }
    };
}

impl_from_for_stdio!(tokio_pipe::PipeWrite);
impl_from_for_stdio!(tokio_pipe::PipeRead);

impl_from_for_stdio!(process::ChildStdin);
impl_from_for_stdio!(process::ChildStdout);
impl_from_for_stdio!(process::ChildStderr);

impl_from_for_stdio!(File);

macro_rules! impl_try_from_tokio_process_child_for_stdio {
    ($type:ident, $wrapper:ty) => {
        impl TryFrom<tokio::process::$type> for Stdio {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                let wrapper: $wrapper = arg.try_into()?;
                Ok(wrapper.0.into())
            }
        }
    };
}

impl_try_from_tokio_process_child_for_stdio!(ChildStdin, ChildInputWrapper);
impl_try_from_tokio_process_child_for_stdio!(ChildStdout, ChildOutputWrapper);
impl_try_from_tokio_process_child_for_stdio!(ChildStderr, ChildOutputWrapper);

/// Input for the remote child.
pub type ChildStdin = tokio_pipe::PipeWrite;

/// Stdout for the remote child.
pub type ChildStdout = tokio_pipe::PipeRead;

/// Stderr for the remote child.
pub type ChildStderr = tokio_pipe::PipeRead;

pub(crate) struct ChildInputWrapper(pub(crate) ChildStdin);
pub(crate) struct ChildOutputWrapper(pub(crate) ChildStderr);

macro_rules! impl_from_impl_child_io {
    (process, $type:ident, $wrapper:ty) => {
        impl TryFrom<tokio::process::$type> for $wrapper {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                let fd = arg.as_raw_fd();

                // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
                let fd = unsafe { dup(fd) }?.into_raw_fd();
                Ok(Self(
                    $type::from_raw_fd_checked(fd).map_err(Error::ChildIo)?,
                ))
            }
        }
    };

    (native_mux, $type:ident, $wrapper:ty) => {
        #[cfg(feature = "native-mux")]
        impl TryFrom<native_mux_impl::$type> for $wrapper {
            type Error = Error;

            fn try_from(arg: native_mux_impl::$type) -> Result<Self, Self::Error> {
                Ok(Self(arg))
            }
        }
    };
}

impl_from_impl_child_io!(process, ChildStdin, ChildInputWrapper);
impl_from_impl_child_io!(process, ChildStdout, ChildOutputWrapper);
impl_from_impl_child_io!(process, ChildStderr, ChildOutputWrapper);

impl_from_impl_child_io!(native_mux, ChildStdin, ChildInputWrapper);
impl_from_impl_child_io!(native_mux, ChildStdout, ChildOutputWrapper);
