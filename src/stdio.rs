use super::Error;

#[cfg(feature = "native-mux")]
use super::native_mux_impl;

use std::fs::File;
use std::io;
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
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
    /// Read/Write to custom fd
    Fd(OwnedFd, bool),
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
    ///
    /// NOTE that the stdio fd must be in blocking mode, otherwise
    /// ssh might not flush all output since it considers
    /// (`EAGAIN`/`EWOULDBLOCK`) as an error
    pub const fn inherit() -> Self {
        Self(StdioImpl::Inherit)
    }

    /// `Stdio::from_raw_fd_owned` takes ownership of the fd passed in
    /// and closes the fd on drop.
    ///
    /// NOTE that the fd will be put into blocking mode, then it will be
    /// closed when `Stdio` is dropped.
    ///
    /// # Safety
    ///
    /// * `fd` - must be a valid fd and must give its ownership to `Stdio`.
    pub unsafe fn from_raw_fd_owned(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(OwnedFd::from_raw_fd(fd), true))
    }
}
/// FromRawFd takes ownership of the fd passed in
/// and closes the fd on drop.
///
/// NOTE that the fd must be in blocking mode, otherwise
/// ssh might not flush all output since it considers
/// (`EAGAIN`/`EWOULDBLOCK`) as an error
#[allow(useless_deprecated)]
#[deprecated(since = "0.9.8", note = "Use Stdio::from_raw_fd_owned instead")]
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(OwnedFd::from_raw_fd(fd), false))
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
            StdioImpl::Fd(fd, _) => unsafe {
                process::Stdio::from_raw_fd(IntoRawFd::into_raw_fd(fd))
            },
        }
    }
}

impl From<OwnedFd> for Stdio {
    fn from(fd: OwnedFd) -> Self {
        Self(StdioImpl::Fd(fd, true))
    }
}

macro_rules! impl_from_for_stdio {
    ($type:ty) => {
        impl From<$type> for Stdio {
            fn from(arg: $type) -> Self {
                let fd = arg.into_raw_fd();
                // safety: $type must have a valid into_raw_fd implementation
                // and must not be RawFd.
                Self(StdioImpl::Fd(unsafe { OwnedFd::from_raw_fd(fd) }, true))
            }
        }
    };
    (deprecated $type:ty) => {
        #[allow(useless_deprecated)]
        #[deprecated(
            since = "0.9.8",
            note = "Use From<OwnedFd> for Stdio or Stdio::from_raw_fd_owned instead"
        )]
        /// deprecated, use `From<OwnedFd> for Stdio` or
        /// [`Stdio::from_raw_fd_owned`] instead.
        impl From<$type> for Stdio {
            fn from(arg: $type) -> Self {
                let fd = arg.into_raw_fd();
                // safety: $type must have a valid into_raw_fd implementation
                // and must not be RawFd.
                Self(StdioImpl::Fd(unsafe { OwnedFd::from_raw_fd(fd) }, true))
            }
        }
    };
}

impl_from_for_stdio!(deprecated tokio_pipe::PipeWrite);
impl_from_for_stdio!(deprecated tokio_pipe::PipeRead);

impl_from_for_stdio!(process::ChildStdin);
impl_from_for_stdio!(process::ChildStdout);
impl_from_for_stdio!(process::ChildStderr);

impl_from_for_stdio!(ChildStdin);
impl_from_for_stdio!(ChildStdout);
impl_from_for_stdio!(ChildStderr);

impl_from_for_stdio!(File);

macro_rules! impl_try_from_tokio_process_child_for_stdio {
    ($type:ident) => {
        impl TryFrom<tokio::process::$type> for Stdio {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                let wrapper: $type = TryFromChildIo::try_from(arg)?;
                Ok(wrapper.0.into())
            }
        }
    };
}

impl_try_from_tokio_process_child_for_stdio!(ChildStdin);
impl_try_from_tokio_process_child_for_stdio!(ChildStdout);
impl_try_from_tokio_process_child_for_stdio!(ChildStderr);

/// Input for the remote child.
#[derive(Debug)]
pub struct ChildStdin(tokio_pipe::PipeWrite);

/// Stdout for the remote child.
#[derive(Debug)]
pub struct ChildStdout(tokio_pipe::PipeRead);

/// Stderr for the remote child.
#[derive(Debug)]
pub struct ChildStderr(tokio_pipe::PipeRead);

pub(crate) trait TryFromChildIo<T>: Sized {
    type Error;

    fn try_from(arg: T) -> Result<Self, Self::Error>;
}

macro_rules! impl_from_impl_child_io {
    (process, $type:ident, $inner:ty) => {
        impl TryFromChildIo<tokio::process::$type> for $type {
            type Error = Error;

            fn try_from(arg: tokio::process::$type) -> Result<Self, Self::Error> {
                let fd = arg.as_raw_fd();

                // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
                let fd = unsafe { BorrowedFd::borrow_raw(fd) };

                let fd = fd
                    .try_clone_to_owned()
                    .map_err(Error::ChildIo)?
                    .into_raw_fd();

                <$inner>::from_raw_fd_checked(fd)
                    .map(Self)
                    .map_err(Error::ChildIo)
            }
        }
    };

    (native_mux, $type:ident) => {
        #[cfg(feature = "native-mux")]
        impl TryFromChildIo<native_mux_impl::$type> for $type {
            type Error = Error;

            fn try_from(arg: native_mux_impl::$type) -> Result<Self, Self::Error> {
                Ok(Self(arg))
            }
        }
    };
}

impl_from_impl_child_io!(process, ChildStdin, tokio_pipe::PipeWrite);
impl_from_impl_child_io!(process, ChildStdout, tokio_pipe::PipeRead);
impl_from_impl_child_io!(process, ChildStderr, tokio_pipe::PipeRead);

impl_from_impl_child_io!(native_mux, ChildStdin);
impl_from_impl_child_io!(native_mux, ChildStdout);
impl_from_impl_child_io!(native_mux, ChildStderr);

macro_rules! impl_child_stdio {
    (AsRawFd, $type:ty) => {
        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                self.0.as_raw_fd()
            }
        }
    };

    (IntoRawFd, $type:ty) => {
        impl IntoRawFd for $type {
            fn into_raw_fd(self) -> RawFd {
                self.0.into_raw_fd()
            }
        }
    };

    (AsyncRead, $type:ty) => {
        impl_child_stdio!(AsRawFd, $type);
        impl_child_stdio!(IntoRawFd, $type);

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
        impl_child_stdio!(AsRawFd, $type);
        impl_child_stdio!(IntoRawFd, $type);

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
