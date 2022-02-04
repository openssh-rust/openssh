#[allow(unused_imports)]
use crate::*;

/// It provides new functions like [`Session::connect_mux`] and
/// [`SessionBuilder::connect_mux`] which communicates with the ssh multiplex
/// master directly, through the control socket, instead of spawning a new
/// process to communicate with it.
///
/// The advantage of this is more robust error reporting, better performance
/// and less memory usage.
///
/// The old implementation (`process-mux`) checks the exit status of `ssh` for
/// indication of error, then parse the output of it and the output of the ssh
/// multiplex master to return an error.
///
/// This method is obviously not so robust as `native-mux`, which directly
/// communicates with ssh multiplex master through its [multiplex protocol].
///
/// The better performance and less memory usage part is mostly because we avoid
/// creating a new process for every command you spawned on remote, every
/// [`Session::check`] and every [`Session::request_port_forward`].
///
/// The new release also add new function [`Session::request_port_forward`],
/// which supports local/remote forwarding of tcp and unix socket stream.
///
/// There are also other changes to API:
///  - A new type [`Stdio`] is used for setting stdin/stdout/stderr.
///  - `ChildStd*` types are now alias for [`tokio_pipe::PipeRead`],
///    [`tokio_pipe::PipeWrite`].
///  - [`Command::spawn`] and [`Command::status`] now confirms to
///    [`std::process::Command`] and [`tokio::process::Command`], in which
///    stdin, stdout and stderr are inherit by default.
///  - [`Command::spawn`] is now an `async` method.
///  - [`RemoteChild::wait`] now takes `self` by value.
///  - [`Error`] is now marked `#[non_exhaustive]` and new variants is added.
///
///[multiplex protocol]: https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.mux
pub mod v0_9_0_rc_1 {}
