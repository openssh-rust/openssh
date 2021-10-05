use super::Fd;
use super::{Error, Result};

use core::mem::replace;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io::{self, IoSlice};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use std::process;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_pipe::{pipe, PipeRead, PipeWrite};

#[derive(Debug)]
enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(Fd),
}

/// Similar to std::process::Stdio
#[derive(Debug)]
pub struct Stdio(StdioImpl);
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
    pub(crate) fn take(&mut self) -> Self {
        replace(self, Stdio::null())
    }

    pub(crate) fn into_stdin(self) -> Result<(Option<Fd>, Option<ChildStdin>)> {
        match self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok((Some(read.into()), Some(ChildStdin(write))))
            }
            StdioImpl::Fd(fd) => Ok((Some(fd), None)),
        }
    }

    pub(crate) fn into_stdout(self) -> Result<(Option<Fd>, Option<ChildStdout>)> {
        match self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok((Some(write.into()), Some(ChildStdout(read))))
            }
            StdioImpl::Fd(fd) => Ok((Some(fd), None)),
        }
    }

    pub(crate) fn into_stderr(self) -> Result<(Option<Fd>, Option<ChildStderr>)> {
        let (fd, stdout) = self.into_stdout()?;
        Ok((fd, stdout.map(|out| ChildStderr(out.0))))
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
                process::Stdio::from_raw_fd(fd.into_raw_fd())
            },
        }
    }
}

/// Input for the remote child.
#[derive(Debug)]
pub struct ChildStdin(PipeWrite);
impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        AsRawFd::as_raw_fd(&self.0)
    }
}
impl IntoRawFd for ChildStdin {
    fn into_raw_fd(self) -> RawFd {
        IntoRawFd::into_raw_fd(self.0)
    }
}
impl AsyncWrite for ChildStdin {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write_vectored(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        AsyncWrite::is_write_vectored(&self.0)
    }
}

macro_rules! impl_reader {
    ( $type:ident ) => {
        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                AsRawFd::as_raw_fd(&self.0)
            }
        }
        impl IntoRawFd for $type {
            fn into_raw_fd(self) -> RawFd {
                IntoRawFd::into_raw_fd(self.0)
            }
        }
        impl AsyncRead for $type {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                AsyncRead::poll_read(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
            }
        }
    };
}

/// stdout for the remote child.
#[derive(Debug)]
pub struct ChildStdout(PipeRead);
impl_reader!(ChildStdout);

/// stderr for the remote child.
#[derive(Debug)]
pub struct ChildStderr(PipeRead);
impl_reader!(ChildStderr);
