use super::Error;
use super::{Fd, Stdio};

use core::pin::Pin;
use core::result;
use core::task::{Context, Poll};

use std::io::{IoSlice, Result};
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_pipe::{pipe, PipeRead, PipeWrite};

use crate::stdio::StdioImpl;

impl Stdio {
    pub(crate) fn into_stdin(self) -> result::Result<(Option<Fd>, Option<ChildStdin>), Error> {
        match self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok((Some(read.into()), Some(ChildStdin(write))))
            }
            StdioImpl::Fd(fd) => Ok((Some(fd), None)),
        }
    }

    pub(crate) fn into_stdout(self) -> result::Result<(Option<Fd>, Option<ChildStdout>), Error> {
        match self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok((Some(write.into()), Some(ChildStdout(read))))
            }
            StdioImpl::Fd(fd) => Ok((Some(fd), None)),
        }
    }

    pub(crate) fn into_stderr(self) -> result::Result<(Option<Fd>, Option<ChildStderr>), Error> {
        let (fd, stdout) = self.into_stdout()?;
        Ok((fd, stdout.map(|out| ChildStderr(out.0))))
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
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        AsyncWrite::poll_write(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        AsyncWrite::poll_flush(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        AsyncWrite::poll_shutdown(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize>> {
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
        impl AsyncRead for $type {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<Result<()>> {
                AsyncRead::poll_read(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
            }
        }
        impl IntoRawFd for $type {
            fn into_raw_fd(self) -> RawFd {
                IntoRawFd::into_raw_fd(self.0)
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
