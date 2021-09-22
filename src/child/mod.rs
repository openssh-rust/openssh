use super::{Error, Session};

#[cfg(not(feature = "enable-openssh-mux-client"))]
mod process_based_impl;
#[cfg(not(feature = "enable-openssh-mux-client"))]
pub use process_based_impl::*;

#[cfg(feature = "enable-openssh-mux-client")]
mod mux_client_based_impl;
#[cfg(feature = "enable-openssh-mux-client")]
pub use mux_client_based_impl::*;

#[cfg(feature = "enable-openssh-mux-client")]
use super::{ChildStderr, ChildStdin, ChildStdout};
