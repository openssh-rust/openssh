use crate::Error;

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, RawHandle};
use std::pin::Pin;
use std::process;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Debug)]
pub(crate) enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Inherit stdin/stdout/stderr
    Inherit,

    /// Read/Write to custom handle
    StdStdio(std::process::Stdio),
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
impl FromRawHandle for Stdio {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        Self(StdioImpl::StdStdio(Stdio::from_raw_handle(handle)))
    }
}
impl From<Stdio> for process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            StdioImpl::Null => process::Stdio::null(),
            StdioImpl::Pipe => process::Stdio::piped(),
            StdioImpl::Inherit => process::Stdio::inherit(),

            StdioImpl::StdStdio(stdio) => stdio,
        }
    }
}

macro_rules! impl_from_for_stdio {
    ($type:ty) => {
        impl From<$type> for Stdio {
            fn from(arg: $type) -> Self {
                let handle = arg.into_raw_handle();
                // safety: $type must have a valid into_raw_handle implementation
                // and must not be RawHandle.
                unsafe { Self::from_raw_handle(handle) }
            }
        }
    };
}

impl_from_for_stdio!(process::ChildStdin);
impl_from_for_stdio!(process::ChildStdout);
impl_from_for_stdio!(process::ChildStderr);

impl_from_for_stdio!(File);

macro_rules! impl_try_from_child_io_for_stdio {
    ($type:ident) => {
        impl TryFrom<tokio::process::$type> for Stdio {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                arg.try_into()
                    .map(StdioImpl::StdStdio)
                    .map(Stdio)
                    .map_err(Error::ChildIo)
            }
        }

        impl TryFrom<$type> for Stdio {
            type Error = Error;

            fn try_from(arg: $type) -> Result<Self, Self::Error> {
                arg.0.try_into()
            }
        }
    };
}

impl_try_from_child_io_for_stdio!(ChildStdin);
impl_try_from_child_io_for_stdio!(ChildStdout);
impl_try_from_child_io_for_stdio!(ChildStderr);

/// Input for the remote child.
#[derive(Debug)]
pub struct ChildStdin(tokio::process::ChildStdin);

/// Stdout for the remote child.
#[derive(Debug)]
pub struct ChildStdout(tokio::process::ChildStdout);

/// Stderr for the remote child.
#[derive(Debug)]
pub struct ChildStderr(tokio::process::ChildStderr);

pub(crate) trait TryFromChildIo<T>: Sized {
    type Error;

    fn try_from(arg: T) -> Result<Self, Self::Error>;
}

macro_rules! impl_from_impl_child_io {
    ($type:ident) => {
        impl TryFromChildIo<tokio::process::$type> for $type {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                Ok(Self(arg))
            }
        }
    };
}

impl_from_impl_child_io!(ChildStdin);
impl_from_impl_child_io!(ChildStdout);
impl_from_impl_child_io!(ChildStderr);

macro_rules! impl_child_stdio {
    (AsRawHandle, $type:ty) => {
        impl AsRawHandle for $type {
            fn as_raw_handle(&self) -> RawHandle {
                self.0.as_raw_handle()
            }
        }
    };

    (AsyncRead, $type:ty) => {
        impl_child_stdio!(AsRawHandle, $type);

        impl AsyncRead for $type {
            fn poll_read(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Pin::new(&mut self.0).poll_read(cx, buf)
            }
        }
    };

    (AsyncWrite, $type: ty) => {
        impl_child_stdio!(AsRawHandle, $type);

        impl AsyncWrite for $type {
            fn poll_write(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                Pin::new(&mut self.0).poll_write(cx, buf)
            }

            fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Pin::new(&mut self.0).poll_flush(cx)
            }

            fn poll_shutdown(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<io::Result<()>> {
                Pin::new(&mut self.0).poll_shutdown(cx)
            }

            fn poll_write_vectored(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                bufs: &[io::IoSlice<'_>],
            ) -> Poll<io::Result<usize>> {
                Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
            }

            fn is_write_vectored(&self) -> bool {
                self.0.is_write_vectored()
            }
        }
    };
}

impl_child_stdio!(AsyncWrite, ChildStdin);
impl_child_stdio!(AsyncRead, ChildStdout);
impl_child_stdio!(AsyncRead, ChildStderr);
