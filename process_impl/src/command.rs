use super::RemoteChild;
use super::{Error, Session};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::io;
use std::process::Stdio;
use tokio::process;

/// A remote process builder, providing fine-grained control over how a new remote process should
/// be spawned.
///
/// A default configuration can be generated using [`Session::command(program)`](Session::command),
/// where `program` gives a path to the program to be executed. Additional builder methods allow
/// the configuration to be changed (for example, by adding arguments) prior to spawning.  The
/// interface is almost identical to that of [`std::process::Command`].
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
    session: &'s Session,
    builder: process::Command,
    stdin_set: bool,
    stdout_set: bool,
    stderr_set: bool,
}

impl<'s> Command<'s> {
    pub(crate) fn new(session: &'s Session, prefix: process::Command) -> Self {
        Self {
            session,
            builder: prefix,
            stdin_set: false,
            stdout_set: false,
            stderr_set: false,
        }
    }
}

impl<'s> Command<'s> {
    /// Adds an argument to pass to the remote program.
    ///
    /// Before it is passed to the remote host, `arg` is escaped so that special characters aren't
    /// evaluated by the remote shell. If you do not want this behavior, use [`raw_arg`].
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
    /// To pass multiple arguments see [`args`](Command::args).
    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.raw_arg(&*shell_escape::unix::escape(Cow::Borrowed(arg.as_ref())));
        self
    }

    /// Adds an argument to pass to the remote program.
    ///
    /// Unlike [`arg`], this method does not shell-escape `arg`. The argument is passed as written
    /// to `ssh`, which will pass it again as an argument to the remote shell. Since the remote
    /// shell may do argument parsing, characters such as spaces and `*` may be interpreted by the
    /// remote shell.
    ///
    /// To pass multiple unescaped arguments see [`raw_args`](Command::raw_args).
    pub fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.builder.arg(arg);
        self
    }

    /// Adds multiple arguments to pass to the remote program.
    ///
    /// Before they are passed to the remote host, each argument in `args` is escaped so that
    /// special characters aren't evaluated by the remote shell. If you do not want this behavior,
    /// use [`raw_args`].
    ///
    /// To pass a single argument see [`arg`](Command::arg).
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.builder
                .arg(&*shell_escape::unix::escape(Cow::Borrowed(arg.as_ref())));
        }
        self
    }

    /// Adds multiple arguments to pass to the remote program.
    ///
    /// Unlike [`args`], this method does not shell-escape `args`. The arguments are passed as
    /// written to `ssh`, which will pass them again as arguments to the remote shell. However,
    /// since the remote shell may do argument parsing, characters such as spaces and `*` may be
    /// interpreted by the remote shell.
    ///
    /// To pass a single argument see [`raw_arg`](Command::raw_arg).
    pub fn raw_args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.builder.args(args);
        self
    }

    /// Configuration for the remote process's standard input (stdin) handle.
    ///
    /// Defaults to [`null`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`null`]: struct.Stdio.html#method.null
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdin(cfg);
        self.stdin_set = true;
        self
    }

    /// Configuration for the remote process's standard output (stdout) handle.
    ///
    /// Defaults to [`null`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`null`]: struct.Stdio.html#method.null
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdout(cfg);
        self.stdout_set = true;
        self
    }

    /// Configuration for the remote process's standard error (stderr) handle.
    ///
    /// Defaults to [`null`] when used with `spawn` or `status`, and
    /// defaults to [`piped`] when used with `output`.
    ///
    /// [`null`]: struct.Stdio.html#method.null
    /// [`piped`]: struct.Stdio.html#method.piped
    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stderr(cfg);
        self.stderr_set = true;
        self
    }

    /// Executes the remote command without waiting for it, returning a handle to it instead.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub fn spawn(&mut self) -> Result<RemoteChild<'s>, Error> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::null());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::null());
        }
        // Then launch!
        let child = self.builder.spawn().map_err(Error::Ssh)?;

        Ok(RemoteChild {
            session: self.session,
            channel: Some(child),
        })
    }

    /// Executes the remote command, waiting for it to finish and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the resulting output).
    /// Stdin is set to `Stdio::null`, and any attempt by the child process to read from
    /// the stdin stream will result in the stream immediately closing.
    pub async fn output(&mut self) -> Result<std::process::Output, Error> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::piped());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::piped());
        }
        // Then launch!
        let output = self.builder.output().await.map_err(Error::Ssh)?;
        match output.status.code() {
            Some(255) => Err(Error::Disconnected),
            Some(127) => Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                &*String::from_utf8_lossy(&output.stderr),
            ))),
            _ => Ok(output),
        }
    }

    /// Executes the remote command, waiting for it to finish and collecting its exit status.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub async fn status(&mut self) -> Result<std::process::ExitStatus, Error> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::null());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::null());
        }
        // Then launch!
        let status = self.builder.status().await.map_err(Error::Ssh)?;
        match status.code() {
            Some(255) => Err(Error::Disconnected),
            Some(127) => Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                "remote command not found",
            ))),
            _ => Ok(status),
        }
    }
}
