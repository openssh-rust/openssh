use super::fd::{dup, Fd};
use super::{process_impl, Error};

#[cfg(feature = "native-mux")]
use super::native_mux_impl;

use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::process;

use std::convert::TryFrom;

#[derive(Debug)]
pub(crate) enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(Fd),
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
}
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(Fd::from_raw_fd(fd)))
    }
}
impl From<Stdio> for process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            StdioImpl::Null => process::Stdio::null(),
            StdioImpl::Pipe => process::Stdio::piped(),

            // safety: StdioImpl(fd) is only constructed from known-valid and
            // owned file descriptors by virtue of the safety requirement
            // for invoking from_raw_fd.
            StdioImpl::Fd(fd) => unsafe { process::Stdio::from_raw_fd(IntoRawFd::into_raw_fd(fd)) },
        }
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

/// Input for the remote child.
pub type ChildStdin = tokio_pipe::PipeWrite;

/// Stdout for the remote child.
pub type ChildStdout = tokio_pipe::PipeRead;

/// Stderr for the remote child.
pub type ChildStderr = tokio_pipe::PipeRead;

pub(crate) struct ChildInputWrapper(pub(crate) ChildStdin);
pub(crate) struct ChildOutputWrapper(pub(crate) ChildStderr);

macro_rules! impl_from_impl_child_io {
    ($type:ident, $wrapper:ident) => {
        impl TryFrom<process_impl::$type> for $wrapper {
            type Error = Error;

            fn try_from(arg: process_impl::$type) -> Result<Self, Self::Error> {
                let fd = arg.as_raw_fd();

                // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
                let fd = unsafe { dup(fd) }?.into_raw_fd();
                // safety: under unix, ChildStdin/ChildStdout is implemented using pipe
                Ok(Self(unsafe { $type::from_raw_fd(fd) }))
            }
        }

        #[cfg(feature = "native-mux")]
        impl TryFrom<native_mux_impl::$type> for $wrapper {
            type Error = Error;

            fn try_from(arg: native_mux_impl::$type) -> Result<Self, Self::Error> {
                Ok(Self(arg))
            }
        }
    };
}

impl_from_impl_child_io!(ChildStdin, ChildInputWrapper);
impl_from_impl_child_io!(ChildStdout, ChildOutputWrapper);

impl TryFrom<process_impl::ChildStderr> for ChildOutputWrapper {
    type Error = Error;

    fn try_from(arg: process_impl::ChildStderr) -> Result<Self, Self::Error> {
        let fd = arg.as_raw_fd();

        // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
        let fd = unsafe { dup(fd) }?.into_raw_fd();
        // safety: under unix, ChildStderr is implemented using pipe
        Ok(Self(unsafe { ChildStderr::from_raw_fd(fd) }))
    }
}
