use super::Error;

use core::pin::Pin;
use core::task::{Context, Poll};

use std::fs::File;
use std::io::{self, IoSlice};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::process;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};

use pin_project::pin_project;

macro_rules! delegate {
    ($impl:expr, $var:ident, $then:block) => {{
        match $impl {
            ProcessImpl($var) => $then,

            #[cfg(feature = "mux_client")]
            MuxClientImpl($var) => $then,
        }
    }};
}

#[derive(Debug)]
pub(crate) enum StdioImpl {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(File),
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
impl<T: IntoRawFd> From<T> for Stdio {
    fn from(val: T) -> Self {
        unsafe { Stdio::from_raw_fd(val.into_raw_fd()) }
    }
}
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(StdioImpl::Fd(File::from_raw_fd(fd)))
    }
}
impl From<Stdio> for process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            StdioImpl::Null => process::Stdio::null(),
            StdioImpl::Pipe => process::Stdio::piped(),
            StdioImpl::Fd(fd) => unsafe { process::Stdio::from_raw_fd(IntoRawFd::into_raw_fd(fd)) },
        }
    }
}

#[pin_project(project = ChildStdinImpProj)]
#[derive(Debug)]
enum ChildStdinImp {
    ProcessImpl(#[pin] tokio::process::ChildStdin),

    #[cfg(feature = "mux_client")]
    MuxClientImpl(#[pin] super::mux_client_impl::ChildStdin),
}

/// Input for the remote child.
#[pin_project]
#[derive(Debug)]
pub struct ChildStdin(#[pin] ChildStdinImp);

impl ChildStdin {
    fn project_enum(self: Pin<&mut Self>) -> ChildStdinImpProj {
        self.project().0.project()
    }
}

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
        use ChildStdinImp::*;

        delegate!(&self.0, imp, { AsRawFd::as_raw_fd(imp) })
    }
}

impl AsyncWrite for ChildStdin {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        use ChildStdinImpProj::*;

        delegate!(self.project_enum(), imp, {
            AsyncWrite::poll_write(imp, cx, buf)
        })
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        use ChildStdinImpProj::*;

        delegate!(self.project_enum(), imp, {
            AsyncWrite::poll_flush(imp, cx)
        })
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        use ChildStdinImpProj::*;

        delegate!(self.project_enum(), imp, {
            AsyncWrite::poll_shutdown(imp, cx)
        })
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        use ChildStdinImpProj::*;

        delegate!(self.project_enum(), imp, {
            AsyncWrite::poll_write_vectored(imp, cx, bufs)
        })
    }

    fn is_write_vectored(&self) -> bool {
        use ChildStdinImp::*;

        delegate!(&self.0, imp, { AsyncWrite::is_write_vectored(imp) })
    }
}

macro_rules! impl_reader {
    ( $type:ident, $imp_type:ident, $imp_proj_type:ident ) => {
        #[pin_project(project = $imp_proj_type)]
        #[derive(Debug)]
        enum $imp_type {
            ProcessImpl(#[pin] tokio::process::$type),

            #[cfg(feature = "mux_client")]
            MuxClientImpl(#[pin] super::mux_client_impl::$type),
        }

        /// Wrapper type for tokio::process and mux_client_impl
        #[pin_project]
        #[derive(Debug)]
        pub struct $type(#[pin] $imp_type);

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
            pub(crate) async fn read_all(&mut self, output: &mut Vec<u8>) -> Result<(), Error> {
                AsyncReadExt::read_to_end(self, output)
                    .await
                    .map_err(Error::ChildIo)?;
                Ok(())
            }
        }

        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                use $imp_type::*;

                delegate!(&self.0, imp, { AsRawFd::as_raw_fd(imp) })
            }
        }

        impl AsyncRead for $type {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<Result<(), io::Error>> {
                use $imp_proj_type::*;

                delegate!(self.project().0.project(), imp, {
                    AsyncRead::poll_read(imp, cx, buf)
                })
            }
        }
    };
}

impl_reader!(ChildStdout, ChildStdoutImp, ChildStdoutImpProj);
impl_reader!(ChildStderr, ChildStderrImp, ChildStderrImpProj);
