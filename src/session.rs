use std::borrow::Cow;
use std::path;

#[cfg(feature = "mux_client")]
pub use openssh_mux_client::connection::{ForwardType, Socket};

pub use super::{Command, KnownHosts, SessionBuilder, Result};

/// A single SSH session to a remote host.
///
/// You can use [`command`] to start a new command on the connected machine.
///
/// When the `Session` is dropped, the connection to the remote host is severed, and any errors
/// silently ignored. To disconnect and be alerted to errors, use [`close`](Session::close).
#[derive(Debug)]
pub struct Session {
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

impl Session {
    fn ctl(&self) -> path::PathBuf {
        todo!()
    }

    /// Return the path to the ssh log file.
    pub fn get_ssh_log_path(&self) -> path::PathBuf {
        todo!()
    }

    /// Connect to the host at the given `addr` over SSH.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    ///
    /// For more options, see [`SessionBuilder`].
    pub async fn connect<S: AsRef<str>>(destination: S, check: KnownHosts) -> Result<Self> 
    {
        todo!()
    }

    /// Check the status of the underlying SSH connection.
    ///
    /// # Cancel safety
    ///
    /// All methods of this struct is not cancellation safe.
    pub async fn check(&self) -> Result<()> {
        todo!()
    }

    /// Constructs a new [`Command`] for launching the program at path `program` on the remote
    /// host.
    ///
    /// Before it is passed to the remote host, `program` is escaped so that special characters
    /// aren't evaluated by the remote shell. If you do not want this behavior, use
    /// [`raw_command`].
    ///
    /// The returned `Command` is a builder, with the following default configuration:
    ///
    /// * No arguments to the program
    /// * Empty stdin and dsicard stdout/stderr for `spawn` or `status`, but create output pipes for
    ///   `output`
    ///
    /// Builder methods are provided to change these defaults and otherwise configure the process.
    ///
    /// If `program` is not an absolute path, the `PATH` will be searched in an OS-defined way on
    /// the host.
    pub fn command<'a, S: Into<Cow<'a, str>>>(&self, program: S) -> Command<'_> {
        todo!()
    }

    /// Constructs a new [`Command`] for launching the program at path `program` on the remote
    /// host.
    ///
    /// Unlike [`command`], this method does not shell-escape `program`, so it may be evaluated in
    /// unforeseen ways by the remote shell.
    ///
    /// The returned `Command` is a builder, with the following default configuration:
    ///
    /// * No arguments to the program
    /// * Empty stdin and dsicard stdout/stderr for `spawn` or `status`, but create output pipes for
    ///   `output`
    ///
    /// Builder methods are provided to change these defaults and otherwise configure the process.
    ///
    /// If `program` is not an absolute path, the `PATH` will be searched in an OS-defined way on
    /// the host.
    pub fn raw_command<'a, S: Into<Cow<'a, str>>>(&self, program: S) -> Command<'_> {
        todo!()
    }

    /// Constructs a new [`Command`] that runs the provided shell command on the remote host.
    ///
    /// The provided command is passed as a single, escaped argument to `sh -c`, and from that
    /// point forward the behavior is up to `sh`. Since this executes a shell command, keep in mind
    /// that you are subject to the shell's rules around argument parsing, such as whitespace
    /// splitting, variable expansion, and other funkyness. I _highly_ recommend you read [this
    /// article] if you observe strange things.
    ///
    /// While the returned `Command` is a builder, like for [`command`], you should not add
    /// additional arguments to it, since the arguments are already passed within the shell
    /// command.
    ///
    /// # Non-standard Remote Shells
    ///
    /// It is worth noting that there are really _two_ shells at work here: the one that sshd
    /// launches for the session, and that launches are command; and the instance of `sh` that we
    /// launch _in_ that session. This method tries hard to ensure that the provided `command` is
    /// passed exactly as-is to `sh`, but this is complicated by the presence of the "outer" shell.
    /// That outer shell may itself perform argument splitting, variable expansion, and the like,
    /// which might produce unintuitive results. For example, the outer shell may try to expand a
    /// variable that is only defined in the inner shell, and simply produce an empty string in the
    /// variable's place by the time it gets to `sh`.
    ///
    /// To counter this, this method assumes that the remote shell (the one launched by `sshd`) is
    /// [POSIX compliant]. This is more or less equivalent to "supports `bash` syntax" if you don't
    /// look too closely. It uses [`shell-escape`] to escape `command` before sending it to the
    /// remote shell, with the expectation that the remote shell will only end up undoing that one
    /// "level" of escaping, thus producing the original `command` as an argument to `sh`. This
    /// works _most of the time_.
    ///
    /// With sufficiently complex or weird commands, the escaping of `shell-escape` may not fully
    /// match the "un-escaping" of the remote shell. This will manifest as escape characters
    /// appearing in the `sh` command that you did not intend to be there. If this happens, try
    /// changing the remote shell if you can, or fall back to [`command`] and do the escaping
    /// manually instead.
    ///
    ///   [POSIX compliant]: https://pubs.opengroup.org/onlinepubs/9699919799/xrat/V4_xcu_chap02.html
    ///   [this article]: https://mywiki.wooledge.org/Arguments
    ///   [`shell-escape`]: https://crates.io/crates/shell-escape
    pub fn shell<S: AsRef<str>>(&self, command: S) -> Command<'_> {
        todo!()
    }

    /// Request to open a local/remote port forwarding.
    /// The `Socket` can be either a unix socket or a tcp socket.
    ///
    /// Currently, there is no way of stopping a port forwarding due to the fact that
    /// openssh multiplex server/master does not support this.
    #[cfg(feature = "mux_client")]
    pub async fn request_port_forward(
        &self,
        forward_type: ForwardType,
        listen_socket: &Socket<'_>,
        connect_socket: &Socket<'_>,
    ) -> Result<()> {
        todo!()
    }

    /// Terminate the remote connection.
    pub async fn close(mut self) -> Result<()> {
        todo!()
    }
}
