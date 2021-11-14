#[cfg(feature = "native_mux")]
use super::native_mux_impl;

use core::fmt;

use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::Path;

/// Type of forwarding
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ForwardType {
    /// Forward requests to a port on the local machine to remote machine.
    Local,

    /// Forward requests to a port on the remote machine to local machine.
    Remote,
}

#[cfg(feature = "native_mux")]
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
    UnixSocket {
        /// Filesystem path
        path: Cow<'a, Path>,
    },

    /// Tcp socket.
    TcpSocket(SocketAddr),
}

#[cfg(feature = "native_mux")]
impl<'a> From<Socket<'a>> for native_mux_impl::Socket<'a> {
    fn from(socket: Socket<'a>) -> Self {
        use native_mux_impl::Socket::*;

        match socket {
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
            Socket::UnixSocket { path } => {
                write!(f, "{}", path.to_string_lossy())
            }
            Socket::TcpSocket(socket) => write!(f, "{}", socket),
        }
    }
}
