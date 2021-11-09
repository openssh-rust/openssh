use super::Error;

use core::mem::ManuallyDrop;

use core::pin::Pin;
use core::task::{Context, Poll};

use std::io::{IoSlice, Result};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use std::process;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

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
}
impl<T: IntoRawFd> From<T> for Stdio {
    fn from(val: T) -> Self {
        unsafe { Stdio::from_raw_fd(val.into_raw_fd()) }
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
            StdioImpl::Fd(fd) => unsafe {
                process::Stdio::from_raw_fd(ManuallyDrop::new(fd).as_raw_fd())
            },
        }
    }
}

#[derive(Debug)]
enum ChildStdinImp {
    ProcessImpl(tokio::process::ChildStdin),

    #[cfg(feature = "mux_client")]
    MuxClientImpl(super::mux_client_impl::ChildStdin),
}

/// Input for the remote child.
#[derive(Debug)]
pub struct ChildStdin(ChildStdinImp);

impl From<tokio::process::ChildStdin> for ChildStdin {
    fn from(imp: tokio::process::ChildStdin) -> Self {
        Self(ChildStdinImp::ProcessImpl(imp))
    }
}

#[cfg(feature = "mux_client")]
impl From<super::mux_client_impl::ChildStdin> for ChildStdin {
    fn from(imp: super::mux_client_impl::ChildStdin) -> Self {
        Self(ChildStdinImp::MuxClientImpl(imp))
    }
}

impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        match &self.0 {
            ChildStdinImp::ProcessImpl(imp) => AsRawFd::as_raw_fd(imp),

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => AsRawFd::as_raw_fd(imp),
        }
    }
}

impl AsyncWrite for ChildStdin {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let inner = unsafe { Pin::into_inner_unchecked(self) };
        match &mut inner.0 {
            ChildStdinImp::ProcessImpl(imp) => {
                AsyncWrite::poll_write(unsafe { Pin::new_unchecked(imp) }, cx, buf)
            }

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => {
                AsyncWrite::poll_write(unsafe { Pin::new_unchecked(imp) }, cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let inner = unsafe { Pin::into_inner_unchecked(self) };
        match &mut inner.0 {
            ChildStdinImp::ProcessImpl(imp) => {
                AsyncWrite::poll_flush(unsafe { Pin::new_unchecked(imp) }, cx)
            }

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => {
                AsyncWrite::poll_flush(unsafe { Pin::new_unchecked(imp) }, cx)
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let inner = unsafe { Pin::into_inner_unchecked(self) };
        match &mut inner.0 {
            ChildStdinImp::ProcessImpl(imp) => {
                AsyncWrite::poll_shutdown(unsafe { Pin::new_unchecked(imp) }, cx)
            }

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => {
                AsyncWrite::poll_shutdown(unsafe { Pin::new_unchecked(imp) }, cx)
            }
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize>> {
        let inner = unsafe { Pin::into_inner_unchecked(self) };
        match &mut inner.0 {
            ChildStdinImp::ProcessImpl(imp) => {
                AsyncWrite::poll_write_vectored(unsafe { Pin::new_unchecked(imp) }, cx, bufs)
            }

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => {
                AsyncWrite::poll_write_vectored(unsafe { Pin::new_unchecked(imp) }, cx, bufs)
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        match &self.0 {
            ChildStdinImp::ProcessImpl(imp) => AsyncWrite::is_write_vectored(imp),

            #[cfg(feature = "mux_client")]
            ChildStdinImp::MuxClientImpl(imp) => AsyncWrite::is_write_vectored(imp),
        }
    }
}

macro_rules! impl_reader {
    ( $type:ident, $imp_type:ident ) => {
        #[derive(Debug)]
        enum $imp_type {
            ProcessImpl(tokio::process::$type),

            #[cfg(feature = "mux_client")]
            MuxClientImpl(super::mux_client_impl::$type),
        }

        /// Wrapper type for tokio::process and mux_client_impl
        #[derive(Debug)]
        pub struct $type($imp_type);

        impl From<tokio::process::$type> for $type {
            fn from(imp: tokio::process::$type) -> Self {
                Self($imp_type::ProcessImpl(imp))
            }
        }

        #[cfg(feature = "mux_client")]
        impl From<super::mux_client_impl::$type> for $type {
            fn from(imp: super::mux_client_impl::$type) -> Self {
                Self($imp_type::MuxClientImpl(imp))
            }
        }

        impl $type {
            pub(crate) async fn read_all(
                &mut self,
                output: &mut Vec<u8>,
            ) -> std::result::Result<(), Error> {
                AsyncReadExt::read_to_end(self, output)
                    .await
                    .map_err(Error::IOError)?;
                Ok(())
            }
        }

        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                match &self.0 {
                    $imp_type::ProcessImpl(imp) => AsRawFd::as_raw_fd(imp),

                    #[cfg(feature = "mux_client")]
                    $imp_type::MuxClientImpl(imp) => AsRawFd::as_raw_fd(imp),
                }
            }
        }

        impl AsyncRead for $type {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<Result<()>> {
                let inner = unsafe { Pin::into_inner_unchecked(self) };
                match &mut inner.0 {
                    $imp_type::ProcessImpl(imp) => {
                        AsyncRead::poll_read(unsafe { Pin::new_unchecked(imp) }, cx, buf)
                    }

                    #[cfg(feature = "mux_client")]
                    $imp_type::MuxClientImpl(imp) => {
                        AsyncRead::poll_read(unsafe { Pin::new_unchecked(imp) }, cx, buf)
                    }
                }
            }
        }
    };
}

impl_reader!(ChildStdout, ChildStdoutImp);
impl_reader!(ChildStderr, ChildStderrImp);
