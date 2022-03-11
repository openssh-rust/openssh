#[cfg(feature = "native-mux")]
use super::native_mux_impl;

#[cfg(feature = "process-mux")]
use std::ffi::OsStr;

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;

/// Type of forwarding
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ForwardType {
    /// Forward requests to a port on the local machine to remote machine.
    Local,

    /// Forward requests to a port on the remote machine to local machine.
    Remote,
}

#[cfg(feature = "native-mux")]
impl From<ForwardType> for native_mux_impl::ForwardType {
    fn from(fwd_type: ForwardType) -> Self {
        use native_mux_impl::ForwardType::*;

        match fwd_type {
            ForwardType::Local => Local,
            ForwardType::Remote => Remote,
        }
    }
}

/// TCP/Unix socket
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Socket<'a> {
    /// Unix socket.
    #[cfg(unix)]
    #[cfg_attr(docsrs, doc(cfg(unix)))]
    UnixSocket {
        /// Filesystem path
        path: Cow<'a, Path>,
    },

    /// Tcp socket.
    TcpSocket(SocketAddr),
}
impl Socket<'_> {
    /// Create a new TcpSocket
    pub fn new<T: ToSocketAddrs>(addr: &T) -> Result<Self, io::Error> {
        let mut it = addr.to_socket_addrs()?;

        let addr = it.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "no more socket addresses to try")
        })?;

        Ok(Socket::TcpSocket(addr))
    }

    #[cfg(feature = "process-mux")]
    pub(crate) fn as_os_str(&self) -> Cow<'_, OsStr> {
        match self {
            #[cfg(unix)]
            Socket::UnixSocket { path } => Cow::Borrowed(path.as_os_str()),
            Socket::TcpSocket(socket) => Cow::Owned(format!("{}", socket).into()),
        }
    }
}

#[cfg(feature = "native-mux")]
impl<'a> From<Socket<'a>> for native_mux_impl::Socket<'a> {
    fn from(socket: Socket<'a>) -> Self {
        use native_mux_impl::Socket::*;

        match socket {
            #[cfg(unix)]
            Socket::UnixSocket { path } => UnixSocket { path },
            Socket::TcpSocket(socket) => TcpSocket {
                port: socket.port() as u32,
                host: socket.ip().to_string().into(),
            },
        }
    }
}

impl<'a> fmt::Display for Socket<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(unix)]
            Socket::UnixSocket { path } => {
                write!(f, "{}", path.to_string_lossy())
            }
            Socket::TcpSocket(socket) => write!(f, "{}", socket),
        }
    }
}
