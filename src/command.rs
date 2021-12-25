use super::stdio::{ChildInputWrapper, ChildOutputWrapper};
use super::RemoteChild;
use super::Stdio;
use super::{Error, Session};

use std::borrow::Cow;
use std::convert::TryFrom;
use std::ffi::OsStr;
use std::marker::PhantomData;
use std::process;

#[derive(Debug)]
pub(crate) enum CommandImp<'s> {
    ProcessImpl(super::process_impl::Command, PhantomData<&'s Session>),

    #[cfg(feature = "native-mux")]
    NativeMuxImpl(super::native_mux_impl::Command<'s>),
}
impl From<super::process_impl::Command> for CommandImp<'_> {
    fn from(imp: super::process_impl::Command) -> Self {
        CommandImp::ProcessImpl(imp, PhantomData)
    }
}

#[cfg(feature = "native-mux")]
impl<'s> From<super::native_mux_impl::Command<'s>> for CommandImp<'s> {
    fn from(imp: super::native_mux_impl::Command<'s>) -> Self {
        CommandImp::NativeMuxImpl(imp)
    }
}

macro_rules! delegate {
    ($impl:expr, $var:ident, $then:block) => {{
        match $impl {
            CommandImp::ProcessImpl($var, _) => $then,

            #[cfg(feature = "native-mux")]
            CommandImp::NativeMuxImpl($var) => $then,
        }
    }};
}

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
    imp: CommandImp<'s>,

    stdout_set: bool,
    stderr_set: bool,
}

impl<'s> Command<'s> {
    pub(crate) fn new(session: &'s super::Session, imp: CommandImp<'s>) -> Self {
        // All implementations of Command initializes stdin, stdout and stderr
        // to Stdio::null()
        Self {
            session,
            imp,

            stdout_set: false,
            stderr_set: false,
        }
    }

    /// Adds an argument to pass to the remote program.
    ///
    /// Before it is passed to the remote host, `arg` is escaped so that special characters aren't
    /// evaluated by the remote shell. If you do not want this behavior, use [`raw_arg`](Command::raw_arg).
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
        self.raw_arg(&*shell_escape::unix::escape(Cow::Borrowed(arg.as_ref())))
    }

    /// Adds an argument to pass to the remote program.
    ///
    /// Unlike [`arg`](Command::arg), this method does not shell-escape `arg`. The argument is passed as written
    /// to `ssh`, which will pass it again as an argument to the remote shell. Since the remote
    /// shell may do argument parsing, characters such as spaces and `*` may be interpreted by the
    /// remote shell.
    ///
    /// To pass multiple unescaped arguments see [`raw_args`](Command::raw_args).
    pub fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        delegate!(&mut self.imp, imp, {
            imp.raw_arg(arg);
        });
        self
    }

    /// Adds multiple arguments to pass to the remote program.
    ///
    /// Before they are passed to the remote host, each argument in `args` is escaped so that
    /// special characters aren't evaluated by the remote shell. If you do not want this behavior,
    /// use [`raw_args`](Command::raw_args).
    ///
    /// To pass a single argument see [`arg`](Command::arg).
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Adds multiple arguments to pass to the remote program.
    ///
    /// Unlike [`args`](Command::args), this method does not shell-escape `args`. The arguments are passed as
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
        delegate!(&mut self.imp, imp, {
            imp.stdin(cfg.into());
        });
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
        delegate!(&mut self.imp, imp, {
            imp.stdout(cfg.into());
        });
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
        delegate!(&mut self.imp, imp, {
            imp.stderr(cfg.into());
        });
        self.stderr_set = true;
        self
    }

    /// Executes the remote command without waiting for it, returning a handle to it
    /// instead.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub async fn spawn(&mut self) -> Result<RemoteChild<'s>, Error> {
        Ok(RemoteChild::new(
            self.session,
            delegate!(&mut self.imp, imp, {
                let (imp, stdin, stdout, stderr) = imp.spawn().await?;
                (
                    imp.into(),
                    stdin
                        .map(TryFrom::try_from)
                        .transpose()?
                        .map(|wrapper: ChildInputWrapper| wrapper.0),
                    stdout
                        .map(TryFrom::try_from)
                        .transpose()?
                        .map(|wrapper: ChildOutputWrapper| wrapper.0),
                    stderr
                        .map(TryFrom::try_from)
                        .transpose()?
                        .map(|wrapper: ChildOutputWrapper| wrapper.0),
                )
            }),
        ))
    }

    /// Executes the remote command, waiting for it to finish and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the resulting
    /// output) and stdin is set to `Stdio::null()`.
    pub async fn output(&mut self) -> Result<process::Output, Error> {
        if !self.stdout_set {
            self.stdout(Stdio::piped());
        }
        if !self.stderr_set {
            self.stderr(Stdio::piped());
        }

        self.spawn().await?.wait_with_output().await
    }

    /// Executes the remote command, waiting for it to finish and collecting its exit status.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub async fn status(&mut self) -> Result<process::ExitStatus, Error> {
        self.spawn().await?.wait().await
    }
}
