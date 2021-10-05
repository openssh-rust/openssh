//! Scriptable SSH through OpenSSH.
//!
//! This crate wraps the OpenSSH remote login client (`ssh` on most machines), and provides
//! a convenient mechanism for running commands on remote hosts. Since all commands are executed
//! through the `ssh` command, all your existing configuration (e.g., in `.ssh/config`) should
//! continue to work as expected.
//!
//! The library's API is modeled closely after that of [`std::process::Command`], since `ssh` also
//! attempts to make the remote process seem as much as possible like a local command. However,
//! there are some differences.
//!
//! First of all, all remote commands are executed in the context of a single ssh
//! [session](Session). Authentication happens once when the session is
//! [established](Session::connect), and subsequent command invocations re-use the same connection.
//! Behind the scenes, the crate uses ssh's [`ControlMaster`] feature to multiplex the channels for
//! the different remote commands. Because of this, each remote command is tied to the lifetime of
//! the [`Session`] that spawned them. When the session is [closed](Session::close), the connection
//! is severed, and there can be no outstanding remote clients.
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
//! And finally, our commands never default to inheriting stdin/stdout/stderr, since we expect you
//! are using this to automate things. Instead, unless otherwise noted, all I/O ports default to
//! [`Stdio::null`](std::process::Stdio::null).
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
//! # #[tokio::main]
//! # async fn main() -> Result<(), mux_client_impl::Error> {
//! use mux_client_impl::{Session, KnownHosts};
//!
//! let session = Session::connect("me@ssh.example.com", KnownHosts::Strict).await?;
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

mod error;
pub use error::Error;

/// process_impl
pub mod process_impl;

/// mux_client_impl
#[cfg(feature = "mux_client")]
pub mod mux_client_impl;

#[cfg(test)]
#[cfg(feature = "mux_client")]
#[macro_use]
extern crate assert_matches;
