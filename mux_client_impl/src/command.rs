use super::RemoteChild;
use super::Result;
use super::{as_raw_fd, ChildStderr, ChildStdin, ChildStdout, Stdio};

use std::path::PathBuf;
use std::process;

use openssh_mux_client::connection::{Connection, EstablishedSession, Session};

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
    session: &'s super::Session,
    cmd: String,
    ctl: PathBuf,

    stdin_v: Stdio,
    stdout_v: Stdio,
    stderr_v: Stdio,
}

impl<'s> Command<'s> {
    pub(crate) fn new(session: &'s super::Session, ctl: PathBuf, cmd: String) -> Self {
        Self {
            session,

            cmd,
            ctl,

            stdin_v: Stdio::null(),
            stdout_v: Stdio::null(),
            stderr_v: Stdio::null(),
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
    /// # fn foo(c: &mut mux_client_impl::Command<'_>) { c
    /// .arg("-C /path/to/repo")
    /// # ; }
    /// ```
    ///
    /// usage would be:
    ///
    /// ```no_run
    /// # fn foo(c: &mut mux_client_impl::Command<'_>) { c
    /// .arg("-C")
    /// .arg("/path/to/repo")
    /// # ; }
    /// ```
    ///
    /// To pass multiple arguments see [`args`](Command::args).
    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.raw_arg(arg)
    }

    /// Adds an argument to pass to the remote program.
    ///
    /// Unlike [`arg`], this method does not shell-escape `arg`. The argument is passed as written
    /// to `ssh`, which will pass it again as an argument to the remote shell. Since the remote
    /// shell may do argument parsing, characters such as spaces and `*` may be interpreted by the
    /// remote shell.
    ///
    /// To pass multiple unescaped arguments see [`raw_args`](Command::raw_args).
    pub fn raw_arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.cmd.push(' ');
        self.cmd.push_str(arg.as_ref());
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
        self.raw_args(args)
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
        S: AsRef<str>,
    {
        for arg in args {
            self.raw_arg(arg);
        }
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
        self.stdin_v = cfg.into();
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
        self.stdout_v = cfg.into();
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
        self.stderr_v = cfg.into();
        self
    }

    /// After this function, stdin, stdout and stderr is reset.
    async fn spawn_impl(
        &mut self,
    ) -> Result<(
        EstablishedSession,
        Option<ChildStdin>,
        Option<ChildStdout>,
        Option<ChildStderr>,
    )> {
        let (stdin, child_stdin) = self.stdin_v.take().into_stdin()?;
        let (stdout, child_stdout) = self.stdout_v.take().into_stdout()?;
        let (stderr, child_stderr) = self.stderr_v.take().into_stderr()?;

        // Then launch!
        let session = Session::builder().cmd(&self.cmd).build();

        let established_session = Connection::connect(&self.ctl)
            .await?
            .open_new_session(
                &session,
                &[as_raw_fd(&stdin), as_raw_fd(&stdout), as_raw_fd(&stderr)],
            )
            .await
            .map_err(|(err, _)| err)?;

        Ok((established_session, child_stdin, child_stdout, child_stderr))
    }

    /// Executes the remote command without waiting for it, returning a handle to it instead.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    ///
    /// After this function, stdin, stdout and stderr is reset.
    pub async fn spawn(&mut self) -> Result<RemoteChild<'s>> {
        let (established_session, child_stdin, child_stdout, child_stderr) =
            self.spawn_impl().await?;

        Ok(RemoteChild::new(
            self.session,
            established_session,
            child_stdin,
            child_stdout,
            child_stderr,
        ))
    }

    /// Executes the remote command, waiting for it to finish and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the resulting output).
    /// Stdin is set to `Stdio::null`, and any attempt by the child process to read from
    /// the stdin stream will result in the stream immediately closing.
    pub async fn output(&mut self) -> Result<process::Output> {
        self.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .await?
            .wait_with_output()
            .await
    }

    /// Executes the remote command, waiting for it to finish and collecting its exit status.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub async fn status(&mut self) -> Result<process::ExitStatus> {
        self.spawn().await?.wait().await
    }
}
