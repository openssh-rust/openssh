#[allow(unused_imports)]
use crate::*;

/// TODO: RENAME THIS INTO THE NEXT VERSION BEFORE RELEASE
///
/// ## Fixed
///  - Fixed crate level doc
///
/// ## Added
///  - Added changelog
///  - Added [`SessionBuilder::compression`]
///  - Associated function [`SessionBuilder::user_known_hosts_file`]
///  - Associated function [`Session::control_socket`] for non-Windows platform.
///
/// ## Changed
///  - Make [`ChildStdin`] an opaque type.
///  - Make [`ChildStdout`] an opaque type.
///  - Make [`ChildStderr`] an opaque type.
///
/// ## Removed
///  - Type `Sftp`.
///  - Type `Mode`.
///  - Type `RemoteFile`.
///  - Associated function `Session::sftp`.
#[doc(hidden)]
pub mod unreleased {}

/// ## Added
///  - Feature flag `native-mux`, an alternative backend that communicates
///    with the ssh multiplex server directly through control socket as opposed
///    `process-mux` implementation that spawns a process to communicate with
///    the ssh multiplex server.
///
///    Compared to `process-mux`, `native-mux` provides more robust error
///    reporting, better performance and reduced memory usage.
///
///    `process-mux` checks the exit status of `ssh` for indication of error,
///    then parse the output of it and the output of the ssh multiplex master
///    to return an error.
///
///    This method is obviously not so robust as `native-mux`, which directly
///    communicates with ssh multiplex master through its [multiplex protocol].
///
///  - Feature flag `process-mux` (enabled by default) to disable the old
///    backend if desired.
///  - API [`Session::connect_mux`] for the new `native-mux` backend,
///    which is used to create a [`Session`] backed by `native-mux`
///    implementation.
///  - API [`SessionBuilder::connect_mux`] for the new `native-mux` backend,
///    which is used to create a [`Session`] backed by `native-mux`
///    implementation.
///  - [`Session::request_port_forward`] for local/remote forwarding
///    of tcp or unix stream sockets, along with [`ForwardType`] and
///    [`Socket`], which is used to setup port forwarding.
///  - A new module [`process`] is added to provide interfaces more similar to
///    [`std::process`].
///  - New variants are added to [`Error`].
///
/// ## Changed
///  - A new type [`Stdio`] is used for setting stdin/stdout/stderr.
///  - [`ChildStdin`], [`ChildStdout`] and [`ChildStderr`] are now aliases
///    for [`tokio_pipe::PipeRead`] and [`tokio_pipe::PipeWrite`].
///  - [`Command::spawn`] and [`Command::status`] now conforms to
///    [`std::process::Command`] and [`tokio::process::Command`], in which
///    stdin, stdout and stderr are inherit by default.
///  - [`Command::spawn`] is now an `async` method.
///  - [`RemoteChild::wait`] now takes `self` by value.
///  - [`Error`] is now marked `#[non_exhaustive]`.
///
/// [multiplex protocol]: https://github.com/openssh/openssh-portable/blob/master/PROTOCOL.mux
pub mod v0_9_0_rc_1 {}
