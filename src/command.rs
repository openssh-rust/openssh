use super::RemoteChild;
use super::{Error, Session};
use std::ffi::OsStr;
use std::process::{self, Stdio};

/// A remote process builder, providing fine-grained control over how a new remote process should
/// be spawned.
///
/// A default configuration can be generated using [`Session::command(program)`](Session::command),
/// where `program` gives a path to the program to be executed. Additional builder methods allow
/// the configuration to be changed (for example, by adding arguments) prior to spawning.
/// The interface is almost identical to that of [`std::process::Command`].
///
/// `Command` can be reused to spawn multiple remote processes. The builder methods change the
/// command without needing to immediately spawn the process. Similarly, you can call builder
/// methods after spawning a process and then spawn a new process with the modified settings.
///
/// # Environment variables and current working directory.
///
/// You'll notice that unlike its `std` counterpart, `Command` does not have any methods for
/// setting environment variables or the current working directory for the remote command. This is
/// because the SSH protocol does not support this (at least not in its standard configuration).
/// For more details on this, see the `ENVIRONMENT` section of [`ssh(1)`]. To work around this,
/// give [`env(1)`] a try. If the remote shell supports it, you can also prefix your command with
/// `["cd", "dir", "&&"]` to run the rest of the command in some directory `dir`.
///
/// # Exit status
///
/// The `ssh` command generally forwards the exit status of the remote process. The exception is if
/// a protocol-level error occured, in which case it will return with exit status 255. Since the
/// remote process _could_ also return with exit status 255, we have no reliable way to distinguish
/// between remote errors and errors from `ssh`, but this library _assumes_ that 255 means the
/// error came from `ssh`, and acts accordingly.
///
///   [`ssh(1)`]: https://linux.die.net/man/1/ssh
///   [`env(1)`]: https://linux.die.net/man/1/env
#[derive(Debug)]
pub struct Command<'s> {
    pub(crate) session: &'s Session,
    pub(crate) builder: process::Command,
}

impl<'s> Command<'s> {
    /// Adds an argument to pass to the remote program.
    ///
    /// The argument is passed as written to `ssh`, which will pass it again as an argument to the
    /// remote shell. However, since the remote shell may do argument parsing, characters such as
    /// spaces and `*` may be interpreted by the remote shell. Since we do not know what shell the
    /// remote host is running, we cannot prevent this, so consider escaping your arguments with
    /// something like [`shellwords`] as necessary.
    ///
    /// Only one argument can be passed per use. So instead of:
    ///
    /// ```no_run
    /// # fn foo(c: &mut openssh::Command<'_>) { c
    /// .arg("-C /path/to/repo")
    /// # ; }
    /// ```
    ///
    /// usage would be:
    ///
    /// ```no_run
    /// # fn foo(c: &mut openssh::Command<'_>) { c
    /// .arg("-C")
    /// .arg("/path/to/repo")
    /// # ; }
    /// ```
    ///
    /// To pass multiple arguments see [`args`].
    ///
    ///   [`shellwords`]: https://crates.io/crates/shellwords
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.builder.arg(arg);
        self
    }

    /// Adds multiple arguments to pass to the remote program.
    ///
    /// The arguments are passed as written to `ssh`, which will pass them again as arguments to
    /// the remote shell. However, since the remote shell may do argument parsing, characters such
    /// as spaces and `*` may be interpreted by the remote shell. Since we do not know what shell
    /// the remote host is running, we cannot prevent this, so consider escaping your arguments
    /// with something like [`shellwords`] as necessary.
    ///
    /// To pass a single argument see [`arg`].
    ///
    ///   [`shellwords`]: https://crates.io/crates/shellwords
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.builder.args(args);
        self
    }

    /// Configuration for the remote process's standard input (stdin) handle.
    ///
    /// Defaults to [`inherit`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`inherit`]: struct.Stdio.html#method.inherit
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdin(cfg);
        self
    }

    /// Configuration for the remote process's standard output (stdout) handle.
    ///
    /// Defaults to [`inherit`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`inherit`]: struct.Stdio.html#method.inherit
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdout(cfg);
        self
    }

    /// Configuration for the remote process's standard error (stderr) handle.
    ///
    /// Defaults to [`inherit`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`inherit`]: struct.Stdio.html#method.inherit
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stderr(cfg);
        self
    }

    /// Executes the remote command without waiting for it, returning a handle to it instead.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    pub fn spawn(&mut self) -> Result<RemoteChild<'s>, Error> {
        let child = self.builder.spawn().map_err(Error::Ssh)?;

        Ok(RemoteChild {
            session: self.session,
            channel: Some(child),
        })
    }

    /// Executes the remote command, waiting for it to finish and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the resulting output).
    /// Stdin is not inherited from the parent and any attempt by the child process to read from
    /// the stdin stream will result in the stream immediately closing.
    pub fn output(&mut self) -> Result<process::Output, Error> {
        let output = self.builder.output().map_err(Error::Ssh)?;
        if let Some(255) = output.status.code() {
            // this is the ssh command's way of telling us that the connection failed
            // TODO: also include output?
            Err(Error::Disconnected)
        } else {
            Ok(output)
        }
    }

    /// Executes the remote command, waiting for it to finish and collecting its exit status.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent.
    pub fn status(&mut self) -> Result<process::ExitStatus, Error> {
        let status = self.builder.status().map_err(Error::Ssh)?;
        if let Some(255) = status.code() {
            // this is the ssh command's way of telling us that the connection failed
            Err(Error::Disconnected)
        } else {
            Ok(status)
        }
    }
}
