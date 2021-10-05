use std::borrow::Cow;
use std::path;

use shell_escape::escape;
use tempfile::TempDir;

use openssh_mux_client::connection::Connection;
pub use openssh_mux_client::connection::{ForwardType, Socket};

mod builder;
pub use builder::{KnownHosts, SessionBuilder};

mod fd;
pub(crate) use fd::*;

mod stdio;
pub use stdio::{ChildStderr, ChildStdin, ChildStdout, Stdio};

mod command;
pub use command::Command;

mod child;
pub use child::RemoteChild;

mod error;
pub use error::Error;
/// Typedef just like std::io::Error
pub type Result<T, Err = Error> = std::result::Result<T, Err>;

/// A single SSH session to a remote host.
///
/// You can use [`command`] to start a new command on the connected machine.
///
/// When the `Session` is dropped, the connection to the remote host is severed, and any errors
/// silently ignored. To disconnect and be alerted to errors, use [`close`](Session::close).
#[derive(Debug)]
pub struct Session {
    /// TempDir will automatically removes the temporary dir on drop
    tempdir: Option<TempDir>,
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

impl Session {
    fn ctl(&self) -> path::PathBuf {
        self.tempdir.as_ref().unwrap().path().join("master")
    }

    /// Return the path to the ssh log file.
    pub fn get_ssh_log_path(&self) -> path::PathBuf {
        self.tempdir.as_ref().unwrap().path().join("log")
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
    pub async fn connect<S: AsRef<str>>(destination: S, check: KnownHosts) -> Result<Self> {
        let mut s = SessionBuilder::default();
        s.known_hosts_check(check);
        s.connect(destination.as_ref()).await
    }

    /// Check the status of the underlying SSH connection.
    ///
    /// # Cancel safety
    ///
    /// All methods of this struct is not cancellation safe.
    pub async fn check(&self) -> Result<()> {
        Connection::connect(&self.ctl())
            .await?
            .send_alive_check()
            .await?;

        Ok(())
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
        self.raw_command(escape(program.into()))
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
        Command::new(self, self.ctl(), program.into().to_string())
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
        let mut cmd = self.command("sh");
        cmd.arg("-c").arg(command);
        cmd
    }

    /// Request to open a local/remote port forwarding.
    /// The `Socket` can be either a unix socket or a tcp socket.
    ///
    /// Currently, there is no way of stopping a port forwarding due to the fact that
    /// openssh multiplex server/master does not support this.
    pub async fn request_port_forward(
        &self,
        forward_type: ForwardType,
        listen_socket: &Socket<'_>,
        connect_socket: &Socket<'_>,
    ) -> Result<()> {
        Connection::connect(&self.ctl())
            .await?
            .request_port_forward(forward_type, listen_socket, connect_socket)
            .await?;

        Ok(())
    }

    async fn request_server_shutdown(tempdir: &TempDir) -> Result<()> {
        Connection::connect(&tempdir.path().join("master"))
            .await?
            .request_stop_listening()
            .await?;

        Ok(())
    }

    /// Terminate the remote connection.
    pub async fn close(mut self) -> Result<()> {
        // This also set self.tempdir to None so that Drop::drop would do nothing.
        let tempdir = self.tempdir.take().unwrap();

        Self::request_server_shutdown(&tempdir).await?;

        tempdir.close().map_err(Error::RemoveTempDir)?;

        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        use tokio::runtime::{Builder, Handle};

        // Keep tempdir alive until the connection is established
        let tempdir = match self.tempdir.take() {
            Some(tempdir) => tempdir,
            None => return,
        };

        let f = || async move {
            let _ = Self::request_server_shutdown(&tempdir).await;
        };

        if let Ok(handle) = Handle::try_current() {
            handle.spawn(f());
        } else {
            let rt = Builder::new_current_thread() // The new Runtime will use current_thread
                .enable_all() // Enable IO and timer driver if available
                .build() // Build and return Result<Runtime>
                .unwrap();

            rt.block_on(f());
        }
    }
}
