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
//! the documetantation the individual methods.
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
//! If you suspect that the connection has failed, [`Session::check`] _may_ provide you with more
//! information than you got from the failing command, since it does not execute a remote command
//! that might interfere with extracting error messages.
//!
//! # Examples
//!
//! ```rust,no_run
//! # fn main() -> Result<(), openssh::Error> {
//! use openssh::{Session, KnownHosts};
//!
//! let session = Session::connect("me@ssh.example.com", KnownHosts::Strict)?;
//! let ls = session.command("ls").output()?;
//! eprintln!("{}", String::from_utf8(ls.stdout).expect("server output was not valid UTF-8"));
//!
//! let whoami = session.command("whoami").output()?;
//! assert_eq!(whoami.stdout, b"me\n");
//!
//! session.close()?;
//! # Ok(()) }
//! ```
//!
//!   [`ControlMaster`]: https://en.wikibooks.org/wiki/OpenSSH/Cookbook/Multiplexing

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use std::ffi::OsStr;
use std::io::{self, prelude::*};
use std::process::{self, Stdio};
use tempfile::Builder;

mod command;
pub use command::Command;

mod child;
pub use child::RemoteChild;

/// A single SSH session to a remote host.
///
/// You can use [`command`] to start a new command on the connected machine.
///
/// When the `Session` is dropped, the connection to the remote host is severed, and any errors
/// silently ignored. To disconnect and be alerted to errors, use [`close`].
#[derive(Debug)]
pub struct Session {
    ctl: tempfile::TempDir,
    addr: String,
    terminated: bool,
    master: std::sync::Mutex<Option<std::process::Child>>,
}

/// Errors that occur when interacting with a remote process.
#[derive(Debug)]
pub enum Error {
    /// The master connection failed.
    Master(io::Error),
    /// Failed to establish initial connection to the remote host.
    Connect(io::Error),
    /// Failed to run the `ssh` command locally.
    Ssh(io::Error),
    /// The remote process failed.
    Remote(io::Error),
    /// The connection to the remote host was severed.
    ///
    /// Note that this is a best-effort error, and it _may_ instead signify that the remote process
    /// exited with an error code of 255. You should call [`Session::check`] to verify if you get
    /// this error back.
    Disconnected,
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

/// Specifies how the host's key fingerprint should be handled.
#[derive(Debug)]
pub enum KnownHosts {
    /// The host's fingerprint must match what is in the known hosts file.
    ///
    /// If the host is not in the known hosts file, the connection is rejected.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=yes`.
    Strict,
    /// Strict, but if the host is not already in the known hosts file, it will be added.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=accept-new`.
    Add,
    /// Accept whatever key the server provides and add it to the known hosts file.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=no`.
    Accept,
}

impl KnownHosts {
    fn as_option(&self) -> &'static str {
        match *self {
            KnownHosts::Strict => "StrictHostKeyChecking=yes",
            KnownHosts::Add => "StrictHostKeyChecking=accept-new",
            KnownHosts::Accept => "StrictHostKeyChecking=no",
        }
    }
}

impl Session {
    fn ctl_path(&self) -> std::path::PathBuf {
        self.ctl.path().join("master")
    }

    /// Connect to the host at the given `addr` over SSH.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    pub fn connect<S: AsRef<str>>(destination: S, check: KnownHosts) -> Result<Self, Error> {
        let dir = Builder::new()
            .prefix(".ssh-connection")
            .tempdir_in("./")
            .map_err(Error::Master)?;
        let mut destination = destination.as_ref();

        // the "new" ssh://user@host:port form is not supported by all versions of ssh, so we
        // always translate it into the option form.
        let mut user = None;
        let mut port = None;
        if destination.starts_with("ssh://") {
            destination = &destination[6..];
            if let Some(at) = destination.find('@') {
                // specified a username -- extract it:
                user = Some(&destination[..at]);
                destination = &destination[(at + 1)..];
            }
            if let Some(colon) = destination.rfind(':') {
                let p = &destination[(colon + 1)..];
                if p.chars().all(|c| c.is_ascii_digit()) {
                    // user specified a port -- extract it:
                    port = Some(p);
                    destination = &destination[..colon];
                }
            }
        }

        let mut init = process::Command::new("ssh");

        init.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("-S")
            .arg(dir.path().join("master"))
            .arg("-M")
            .arg("-f")
            .arg("-N")
            .arg("-o")
            .arg("ControlPersist=yes")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg(check.as_option());

        if let Some(port) = port {
            init.arg("-p").arg(port);
        }

        if let Some(user) = user {
            init.arg("-l").arg(user);
        }

        init.arg(destination);

        // eprintln!("{:?}", init);

        // we spawn and immediately wait, because the process is supposed to fork.
        // note that we cannot use .output, since it _also_ tries to read all of stdout/stderr.
        // if the call _didn't_ error, then the backgrounded ssh client will still hold onto those
        // handles, and it's still running, so those reads will hang indefinitely.
        let mut child = init.spawn().map_err(Error::Connect)?;
        let status = child.wait().map_err(Error::Connect)?;

        if let Some(255) = status.code() {
            // this is the ssh command's way of telling us that the connection failed
            let mut stderr = String::new();
            child
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();

            return Err(interpret_ssh_error(&stderr));
        }

        Ok(Self {
            ctl: dir,
            addr: String::from(destination),
            terminated: false,
            master: std::sync::Mutex::new(Some(child)),
        })
    }

    /// Check the status of the underlying SSH connection.
    ///
    /// Since this does not run a remote command, it has a better chance of extracting useful error
    /// messages than other commands.
    pub fn check(&self) -> Result<(), Error> {
        if self.terminated {
            return Err(Error::Disconnected);
        }

        let check = process::Command::new("ssh")
            .arg("-S")
            .arg(self.ctl_path())
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-O")
            .arg("check")
            .arg(&self.addr)
            .output()
            .map_err(Error::Ssh)?;

        if let Some(255) = check.status.code() {
            if let Some(master_error) = self.take_master_error() {
                Err(master_error)
            } else {
                Err(Error::Disconnected)
            }
        } else {
            Ok(())
        }
    }

    /// Constructs a new [`Command`] for launching the program at path `program` on the remote
    /// host.
    ///
    /// The returned `Command` is a builder, with the following default configuration:
    ///
    /// * No arguments to the program
    /// * Inherit stdin/stdout/stderr for `spawn` or `status`, but create pipes for `output`
    ///
    /// Builder methods are provided to change these defaults and otherwise configure the process.
    ///
    /// If `program` is not an absolute path, the `PATH` will be searched in an OS-defined way on
    /// the host.
    // TODO: we may want to re-visit the defaults for wait/output/spawn, as it's not clear Inherit
    // as the default makes as much sense in the context of a remote host library?
    pub fn command<S: AsRef<OsStr>>(&self, program: S) -> Command<'_> {
        // XXX: Should we do a self.check() here first?

        let mut cmd = process::Command::new("ssh");
        cmd.arg("-S")
            .arg(self.ctl_path())
            .arg("-T")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(&self.addr)
            .arg(program);

        Command {
            session: self,
            builder: cmd,
        }
    }

    /// Terminate the remote connection.
    pub fn close(mut self) -> Result<(), Error> {
        self.terminate()
    }

    fn take_master_error(&self) -> Option<Error> {
        let mut master = self.master.lock().unwrap().take()?;

        let status = master
            .wait()
            .expect("failed to await master that _we_ spawned");

        if status.success() {
            // master exited cleanly, so we assume that the
            // connection was simply closed by the remote end.
            return None;
        }

        let mut stderr = String::new();
        if let Err(e) = master
            .stderr
            .expect("master was spawned with piped stderr")
            .read_to_string(&mut stderr)
        {
            return Some(Error::Master(e));
        }
        let stderr = stderr.trim();

        Some(Error::Master(io::Error::new(io::ErrorKind::Other, stderr)))
    }

    fn terminate(&mut self) -> Result<(), Error> {
        if !self.terminated {
            let exit = process::Command::new("ssh")
                .arg("-S")
                .arg(self.ctl_path())
                .arg("-o")
                .arg("BatchMode=yes")
                .arg("-O")
                .arg("exit")
                .arg(&self.addr)
                .output()
                .map_err(Error::Ssh)?;

            self.terminated = true;
            if !exit.status.success() {
                if let Some(master_error) = self.take_master_error() {
                    return Err(master_error);
                }

                // let's get this case straight:
                // we tried to tell the master to exit.
                // the command execution did not fail.
                // the command returned a failure exist code.
                // the master did not produce an error.
                // what could cause that?
                //
                // the only thing I can think of at the moment is that the remote end cleanly
                // closed the connection, probably by virtue of being killed (but without the
                // network dropping out). since we were told to _close_ the connection, well, we
                // have succeeded, so this should not produce an error.
                //
                // we will still _collect_ the error that -O exit produced though,
                // just for ease of debugging.

                let _exit_err = String::from_utf8_lossy(&exit.stderr);
                let _err = _exit_err.trim();
                // eprintln!("{}", _err);
            }
        }

        Ok(())
    }
}

fn interpret_ssh_error(stderr: &str) -> Error {
    // we want to turn the string-only ssh error into something a little more "handleable".
    // we do this by trying to interpret the output from `ssh`. this is error-prone, but
    // the best we can do. if you find ways to impove this, even just through heuristics,
    // please file an issue or PR :)
    //
    // format is:
    //
    //     ssh: ssh error: io error
    let mut stderr = stderr.trim();
    if stderr.starts_with("ssh: ") {
        stderr = &stderr["ssh: ".len()..];
    }
    if stderr.starts_with("Warning: Permanently added ") {
        // added to hosts file -- let's ignore that message
        stderr = stderr.splitn(2, "\r\n").nth(1).unwrap_or("");
    }
    let mut kind = io::ErrorKind::ConnectionAborted;
    let mut err = stderr.splitn(2, ": ");
    if let Some(ssh_error) = err.next() {
        if ssh_error.starts_with("Could not resolve") {
            // match what `std` gives: https://github.com/rust-lang/rust/blob/a5de254862477924bcd8b9e1bff7eadd6ffb5e2a/src/libstd/sys/unix/net.rs#L40
            // we _could_ match on "Name or service not known" from io_error,
            // but my guess is that the ssh error is more stable.
            kind = io::ErrorKind::Other;
        }

        if let Some(io_error) = err.next() {
            match io_error {
                "Network is unreachable" => {
                    kind = io::ErrorKind::Other;
                }
                "Connection refused" => {
                    kind = io::ErrorKind::ConnectionRefused;
                }
                e if e.starts_with("Permission denied") => {
                    if ssh_error.starts_with("connect to host") {
                        // this is the macOS version of "network is unreachable".
                        kind = io::ErrorKind::Other;
                    } else {
                        kind = io::ErrorKind::PermissionDenied;
                    }
                }
                _ => {}
            }
        }
    }

    // NOTE: we may want to provide more structured connection errors than just io::Error?
    // NOTE: can we re-use this method for non-connect cases?
    Error::Connect(io::Error::new(kind, stderr))
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self.terminate();
        }
    }
}

#[test]
fn parse_error() {
    let err = "ssh: Warning: Permanently added \'login.csail.mit.edu,128.52.131.0\' (ECDSA) to the list of known hosts.\r\nopenssh-tester@login.csail.mit.edu: Permission denied (publickey,gssapi-keyex,gssapi-with-mic,password,keyboard-interactive).";
    let err = interpret_ssh_error(err);
    let target = io::Error::new(io::ErrorKind::PermissionDenied, "openssh-tester@login.csail.mit.edu: Permission denied (publickey,gssapi-keyex,gssapi-with-mic,password,keyboard-interactive).");
    if let Error::Connect(e) = err {
        assert_eq!(e.kind(), target.kind());
        assert_eq!(format!("{}", e), format!("{}", target));
    } else {
        unreachable!("{:?}", err);
    }
}
