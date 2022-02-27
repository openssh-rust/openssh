//! Scriptable SSH through OpenSSH (**only works on unix**).
//!
//! This crate wraps the OpenSSH remote login client (`ssh` on most machines), and provides
//! a convenient mechanism for running commands on remote hosts. Since all commands are executed
//! through the `ssh` command, all your existing configuration (e.g., in `.ssh/config`) should
//! continue to work as expected.
//!
//! # Executing remote processes
//!
//! The library's API is modeled closely after that of [`std::process::Command`], since `ssh` also
//! attempts to make the remote process seem as much as possible like a local command. However,
//! there are some differences.
//!
//! First of all, all remote commands are executed in the context of a single ssh
//! [session](Session). Authentication happens once when the session is
//! [established](Session::connect), and subsequent command invocations re-use the same connection.
//!
//! Note that the maximum number of multiplexed remote commands is 10 by default. This value can be
//! increased by changing the `MaxSessions` setting in [`sshd_config`].
//!
//! Much like with [`std::process::Command`], you have multiple options when it comes to launching
//! a remote command. You can [spawn](Command::spawn) the remote command, which just gives you a
//! handle to the running process, you can run the command and wait for its
//! [output](Command::output), or you can run it and just extract its [exit
//! status](Command::status). Unlike its `std` counterpart though, these methods on [`Command`] can
//! fail even if the remote command executed successfully, since there is a fallible network
//! separating you from it.
//!
//! Also unlike its `std` counterpart, [`spawn`](Command::spawn) gives you a [`RemoteChild`] rather
//! than a [`std::process::Child`]. Behind the scenes, a remote child is really just a process
//! handle to the _local_ `ssh` instance corresponding to the spawned remote command. The behavior
//! of the methods of [`RemoteChild`] therefore match the behavior of `ssh`, rather than that of
//! the remote command directly. Usually, these are the same, though not always, as highlighted in
//! the documetantation the individual methods. See also the section below on Remote Shells.
//!
//! # Connection modes
//!
//! This library provides two way to connect to the [`ControlMaster`]:
//!
//! One is to spawn a new process, the other is to connect to
//! the control socket directly.
//!
//! The process implementation executes remote commands by invoking
//! the ssh command locally with arguments that make the invocation
//! reuse the connections set up by the control master.
//!
//! This maximizes compatibility with OpenSSH, but loses out on some fidelity
//! in information about execution since only the exit code and the output of
//! the ssh command is available to inspect.
//!
//! The native mux implementation on the other hand connects directly to
//! the ssh control master and executes commands and retrieves the exit codes and
//! the output of the remote process over its native protocol.
//!
//! This gives better access to error information at the cost of introducing
//! more non-OpenSSH code into the call path.
//!
//! The former parses the stdout/stderr of the ssh control master to retrieve the error
//! for any failed operations, while the later retrieves the error from the control socket
//! directly.
//!
//! Thus, the error handling in the later is more robust.
//!
//! Also, the former requires one process to be spawn for every connection while the later only
//! needs to create one socket, so the later has better performance and consumes less resource.
//!
//! Behind the scenes, the crate uses ssh's [`ControlMaster`] feature to multiplex the channels for
//! the different remote commands. Because of this, each remote command is tied to the lifetime of
//! the [`Session`] that spawned them. When the session is [closed](Session::close), the connection
//! is severed, and there can be no outstanding remote clients.
//!
//! # Authentication
//!
//! This library supports only password-less authentication schemes. If running `ssh` to a target
//! host requires you to provide input on standard input (`STDIN`), then this crate will not work
//! for you. You should set up keypair-based authentication instead.
//!
//! # Errors
//!
//! Since we are wrapping the `ssh`, which in turn runs a remote command that we do not control, we
//! do not have a reliable way to tell the difference between what is a failure of the SSH
//! connection itself, and what is a program error from the remote host. We do our best with some
//! heuristics (like `ssh` exiting with status code 255 if a connection error occurs), but the
//! errors from this crate will almost necessarily be worse than those of a native SSH
//! implementation. Sorry in advance :)
//!
//! This also means that you may see strange errors when the remote process is terminated by a
//! signal (such as through `kill` or `pkill`). When this happens, all the local ssh program sees
//! is that the remote process disappeared, and so it returns with an error. It does not
//! communicate that the process exited due to a signal. In cases like this, your call will return
//! [`Error::Disconnected`], because the connection to _that_ remote process was disconnected. The
//! ssh connection as a whole is likely still intact.
//!
//! To check if the connection has truly failed, use [`Session::check`]. It will return `Ok` if the
//! master connection is still operational, and _may_ provide you with more information than you
//! got from the failing command (that is, just [`Error::Disconnected`]) if it is not.
//!
//! # Remote Shells
//!
//! When you invoke a remote command through ssh, the remote command is executed by a shell on the
//! remote end. That shell _interprets_ anything passed to it — it might evalute words starting
//! with `$` as variables, split arguments by whitespace, and other things a shell is wont to do.
//! Since that is _usually_ not what you expect to happen, `.arg("a b")` should pass a _single_
//! argument with the value `a b`, `openssh` _escapes_ every argument (and the command itself) by
//! default using [`shell-escape`]. This works well in most cases, but might run into issues when
//! the remote shell (generally the remote user's login shell) has a different syntax than the
//! shell `shell-escape` targets (bash). For example, Windows shells have different escaping syntax
//! than bash does.
//!
//! If this applies to you, you can use [`raw_arg`](Command::raw_arg),
//! [`raw_args`](Command::raw_args), and [`raw_command`](Session::raw_command) to bypass the
//! escaping that `openssh` normally does for you.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "native-mux")]
//! # #[tokio::main]
//! # async fn main() -> Result<(), openssh::Error> {
//! use openssh::{Session, KnownHosts};
//!
//! let session = Session::connect_mux("me@ssh.example.com", KnownHosts::Strict).await?;
//!
//! let ls = session.command("ls").output().await?;
//! eprintln!("{}", String::from_utf8(ls.stdout).expect("server output was not valid UTF-8"));
//!
//! let whoami = session.command("whoami").output().await?;
//! assert_eq!(whoami.stdout, b"me\n");
//!
//! session.close().await?;
//! # Ok(()) }
//! ```
//!
//!   [`ControlMaster`]: https://en.wikibooks.org/wiki/OpenSSH/Cookbook/Multiplexing
//!   [`sshd_config`]: https://linux.die.net/man/5/sshd_config
//!   [`shell-escape`]: https://crates.io/crates/shell-escape

#![warn(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    rust_2018_idioms,
    unreachable_pub
)]
#![cfg_attr(
    not(any(feature = "process-mux", feature = "native-mux")),
    allow(unused_variables, unreachable_code, unused_imports, dead_code)
)]
// only enables the nightly `doc_cfg` feature when
// the `docsrs` configuration attribute is defined
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(not(unix))]
compile_error!("This crate can only be used on unix");

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::Path;

mod stdio;
pub use stdio::{ChildStderr, ChildStdin, ChildStdout, Stdio};

mod builder;
pub use builder::{KnownHosts, SessionBuilder};

mod command;
pub use command::Command;

mod child;
pub use child::RemoteChild;

mod error;
pub use error::Error;

mod sftp;
pub use sftp::{Mode, RemoteFile, Sftp};

#[cfg(feature = "process-mux")]
pub(crate) mod process_impl;

#[cfg(feature = "native-mux")]
pub(crate) mod native_mux_impl;

#[cfg(doc)]
/// Changelog for this crate.
pub mod changelog;

mod port_forwarding;
pub use port_forwarding::*;

/// Types to create and interact with the Remote Process
pub mod process {
    pub use super::{ChildStderr, ChildStdin, ChildStdout, Command, RemoteChild, Stdio};
}

#[derive(Debug)]
pub(crate) enum SessionImp {
    #[cfg(feature = "process-mux")]
    ProcessImpl(process_impl::Session),

    #[cfg(feature = "native-mux")]
    NativeMuxImpl(native_mux_impl::Session),
}

#[cfg(any(feature = "process-mux", feature = "native-mux"))]
macro_rules! delegate {
    ($impl:expr, $var:ident, $then:block) => {{
        match $impl {
            #[cfg(feature = "process-mux")]
            SessionImp::ProcessImpl($var) => $then,

            #[cfg(feature = "native-mux")]
            SessionImp::NativeMuxImpl($var) => $then,
        }
    }};
}

#[cfg(not(any(feature = "process-mux", feature = "native-mux")))]
macro_rules! delegate {
    ($impl:expr, $var:ident, $then:block) => {{
        unreachable!("Neither feature process-mux nor native-mux is enabled")
    }};
}

/// A single SSH session to a remote host.
///
/// You can use [`command`](Session::command) to start a new command on the connected machine.
///
/// When the `Session` is dropped, the connection to the remote host is severed, and any errors
/// silently ignored. To disconnect and be alerted to errors, use [`close`](Session::close).
#[derive(Debug)]
pub struct Session(SessionImp);

#[cfg(feature = "process-mux")]
impl From<process_impl::Session> for Session {
    fn from(imp: process_impl::Session) -> Self {
        Self(SessionImp::ProcessImpl(imp))
    }
}

#[cfg(feature = "native-mux")]
impl From<native_mux_impl::Session> for Session {
    fn from(imp: native_mux_impl::Session) -> Self {
        Self(SessionImp::NativeMuxImpl(imp))
    }
}

// TODO: UserKnownHostsFile for custom known host fingerprint.

impl Session {
    /// Connect to the host at the given `host` over SSH using process impl, which will
    /// spawn a new ssh process for each `Child` created.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    ///
    /// For more options, see [`SessionBuilder`].
    #[cfg(feature = "process-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "process-mux")))]
    pub async fn connect<S: AsRef<str>>(destination: S, check: KnownHosts) -> Result<Self, Error> {
        let mut s = SessionBuilder::default();
        s.known_hosts_check(check);
        s.connect(destination.as_ref()).await
    }

    /// Connect to the host at the given `host` over SSH using native mux impl, which
    /// will create a new socket connection for each `Child` created.
    ///
    /// See the crate-level documentation for more details on the difference between native and process-based mux.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    ///
    /// For more options, see [`SessionBuilder`].
    #[cfg(feature = "native-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "native-mux")))]
    pub async fn connect_mux<S: AsRef<str>>(
        destination: S,
        check: KnownHosts,
    ) -> Result<Self, Error> {
        let mut s = SessionBuilder::default();
        s.known_hosts_check(check);
        s.connect_mux(destination.as_ref()).await
    }

    /// Check the status of the underlying SSH connection.
    pub async fn check(&self) -> Result<(), Error> {
        delegate!(&self.0, imp, { imp.check().await })
    }

    /// Get the SSH connection's control socket path.
    pub fn control_socket(&self) -> &Path {
        delegate!(&self.0, imp, { imp.ctl() })
    }

    /// Constructs a new [`Command`] for launching the program at path `program` on the remote
    /// host.
    ///
    /// Before it is passed to the remote host, `program` is escaped so that special characters
    /// aren't evaluated by the remote shell. If you do not want this behavior, use
    /// [`raw_command`](Session::raw_command).
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
        self.raw_command(&*shell_escape::unix::escape(program.into()))
    }

    /// Constructs a new [`Command`] for launching the program at path `program` on the remote
    /// host.
    ///
    /// Unlike [`command`](Session::command), this method does not shell-escape `program`, so it may be evaluated in
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
    pub fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command<'_> {
        Command::new(
            self,
            delegate!(&self.0, imp, { imp.raw_command(program).into() }),
        )
    }

    /// Constructs a new [`Command`] that runs the provided shell command on the remote host.
    ///
    /// The provided command is passed as a single, escaped argument to `sh -c`, and from that
    /// point forward the behavior is up to `sh`. Since this executes a shell command, keep in mind
    /// that you are subject to the shell's rules around argument parsing, such as whitespace
    /// splitting, variable expansion, and other funkyness. I _highly_ recommend you read
    /// [this article] if you observe strange things.
    ///
    /// While the returned `Command` is a builder, like for [`command`](Session::command), you should not add
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
    /// changing the remote shell if you can, or fall back to [`command`](Session::command)
    /// and do the escaping manually instead.
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
    /// If `forward_type` == Local, then `listen_socket` on local machine will be
    /// forwarded to `connect_socket` on remote machine.
    ///
    /// Otherwise, `listen_socket` on the remote machine will be forwarded to `connect_socket`
    /// on the local machine.
    ///
    /// Currently, there is no way of stopping a port forwarding due to the fact that
    /// openssh multiplex server/master does not support this.
    pub async fn request_port_forward(
        &self,
        forward_type: ForwardType,
        listen_socket: Socket<'_>,
        connect_socket: Socket<'_>,
    ) -> Result<(), Error> {
        delegate!(&self.0, imp, {
            imp.request_port_forward(forward_type, listen_socket, connect_socket)
                .await
        })
    }

    /// Prepare to perform file operations on the remote host.
    ///
    /// See [`Sftp`] for details on how to interact with the remote files.
    pub fn sftp(&self) -> Sftp<'_> {
        Sftp::new(self)
    }

    /// Terminate the remote connection.
    pub async fn close(self) -> Result<(), Error> {
        delegate!(self.0, imp, { imp.close().await })
    }
}
