use core::fmt;
use core::num::NonZeroU32;

#[cfg(feature = "mux_client")]
use super::mux_client_impl;

/// Type of forwarding
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ForwardType {
    /// Forward requests to a port on the local machine to remote machine.
    Local,

    /// Forward requests to a port on the remote machine to local machine.
    Remote,
}

#[cfg(feature = "mux_client")]
impl From<ForwardType> for mux_client_impl::ForwardType {
    fn from(fwd_type: ForwardType) -> Self {
        use mux_client_impl::ForwardType::*;

        match fwd_type {
            ForwardType::Local => Local,
            ForwardType::Remote => Remote,
        }
    }
}

/// TCP/Unix socket
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Socket<'a> {
    /// Unix socket.
    UnixSocket {
        /// Filesystem path
        path: &'a str,
    },

    /// Tcp socket.
    TcpSocket {
        /// Port for tcp socket
        port: NonZeroU32,
        /// Hostname, can be any valid ip or hostname.
        host: &'a str,
    },
}

#[cfg(feature = "mux_client")]
impl<'a> From<Socket<'a>> for mux_client_impl::Socket<'a> {
    fn from(socket: Socket<'a>) -> Self {
        use mux_client_impl::Socket::*;

        match socket {
            Socket::UnixSocket { path } => UnixSocket { path },
            Socket::TcpSocket { port, host } => TcpSocket { port, host },
        }
    }
}

impl<'a> fmt::Display for Socket<'a> {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Socket::UnixSocket { path } => write!(f, "{}", path),
            Socket::TcpSocket { host, port } => write!(f, "{}:{}", host, port),
        }
    }
}
