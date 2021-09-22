use super::Error;
use super::RemoteChild;

use core::mem::replace;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::fs::OpenOptions;
use std::io::{self, IoSlice};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;
use nix::unistd;

use openssh_mux_client::connection::{Connection, EstablishedSession, Session};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_pipe::{pipe, PipeRead, PipeWrite};

/// Open "/dev/null" with RW.
fn get_null_fd() -> RawFd {
    static NULL_FD: OnceCell<RawFd> = OnceCell::new();
    *NULL_FD.get_or_init(|| {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .unwrap();
        IntoRawFd::into_raw_fd(file)
    })
}

/// Wrapper for RawFd that automatically closes it on drop.
#[derive(Debug)]
pub struct Fd(RawFd);
impl Drop for Fd {
    fn drop(&mut self) {
        unistd::close(self.0).unwrap();
    }
}
impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
impl<T: IntoRawFd> From<T> for Fd {
    fn from(val: T) -> Self {
        Self(IntoRawFd::into_raw_fd(val))
    }
}
impl FromRawFd for Fd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(fd)
    }
}

fn as_raw_fd(fd: &Option<Fd>) -> RawFd {
    match fd {
        Some(fd) => fd.0,
        None => get_null_fd(),
    }
}

/// Similar to std::process::Stdio
#[derive(Debug)]
pub enum Stdio {
    /// Read/Write to /dev/null
    Null,
    /// Read/Write to a newly created pipe
    Pipe,
    /// Read/Write to custom fd
    Fd(Fd),
}
impl Stdio {
    pub fn piped() -> Self {
        Stdio::Pipe
    }

    fn to_stdin(self) -> Result<(Option<Fd>, Option<ChildStdin>), Error> {
        match self {
            Stdio::Null => Ok((None, None)),
            Stdio::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok(( Some(read.into()), Some(ChildStdin(write)) ))
            }
            Stdio::Fd(fd) => Ok(( Some(fd), None )),
        }
    }

    fn to_stdout(self) -> Result<(Option<Fd>, Option<ChildStdout>), Error> {
        match self {
            Stdio::Null => Ok((None, None)),
            Stdio::Pipe => {
                let (read, write) = pipe().map_err(Error::IOError)?;
                Ok(( Some(write.into()), Some(ChildStdout(read)) ))
            }
            Stdio::Fd(fd) => Ok(( Some(fd), None )),
        }
    }

    fn to_stderr(self) -> Result<(Option<Fd>, Option<ChildStderr>), Error> {
        let (fd, stdout) = self.to_stdout()?;
        Ok(( fd, stdout.map(|out| ChildStderr(out.0)) ))
    }
}
impl<T: IntoRawFd> From<T> for Stdio {
    fn from(val: T) -> Self {
        Stdio::Fd(val.into())
    }
}
impl FromRawFd for Stdio {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Stdio::Fd(fd.into())
    }
}

/// Input for the remote child.
#[derive(Debug)]
pub struct ChildStdin(PipeWrite);
impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        AsRawFd::as_raw_fd(&self.0)
    }
}
impl IntoRawFd for ChildStdin {
    fn into_raw_fd(self) -> RawFd {
        IntoRawFd::into_raw_fd(self.0)
    }
}
impl AsyncWrite for ChildStdin {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write_vectored(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        AsyncWrite::is_write_vectored(&self.0)
    }
}

macro_rules! impl_writer {
    ( $type:ident ) => {
        impl AsRawFd for $type {
            fn as_raw_fd(&self) -> RawFd {
                AsRawFd::as_raw_fd(&self.0)
            }
        }
        impl IntoRawFd for $type {
            fn into_raw_fd(self) -> RawFd {
                IntoRawFd::into_raw_fd(self.0)
            }
        }
        impl AsyncRead for $type {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                AsyncRead::poll_read(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx, buf)
            }
        }
    };
}

/// stdout for the remote child.
#[derive(Debug)]
pub struct ChildStdout(PipeRead);
impl_writer!(ChildStdout);

/// stderr for the remote child.
#[derive(Debug)]
pub struct ChildStderr(PipeRead);
impl_writer!(ChildStderr);

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
    control_path: String,

    stdin_v: Stdio,
    stdout_v: Stdio,
    stderr_v: Stdio,
}

impl<'s> Command<'s> {
    pub(crate) fn new(session: &'s super::Session, control_path: String, cmd: String) -> Self {
        Self {
            session,

            cmd,
            control_path,

            stdin_v: Stdio::Null,
            stdout_v: Stdio::Null,
            stderr_v: Stdio::Null,
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
    ) -> Result<
        (
            EstablishedSession,
            Option<ChildStdin>,
            Option<ChildStdout>,
            Option<ChildStderr>,
        ),
        Error,
    > {
        let (stdin, child_stdin) = replace(&mut self.stdin_v, Stdio::Null).to_stdin()?;
        let (stdout, child_stdout) = replace(&mut self.stdout_v, Stdio::Null).to_stdout()?;
        let (stderr, child_stderr) = replace(&mut self.stderr_v, Stdio::Null).to_stderr()?;

        // Then launch!
        let session = Session::builder().cmd(&self.cmd).build();

        let established_session = Connection::connect(&self.control_path)
            .await?
            .open_new_session(
                &session,
                &[
                    as_raw_fd(&stdin),
                    as_raw_fd(&stdout),
                    as_raw_fd(&stderr)
                ]
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
    pub async fn spawn(&mut self) -> Result<RemoteChild<'s>, Error> {
        let (established_session, child_stdin, child_stdout, child_stderr) =
            self.spawn_impl().await?;

        Ok(RemoteChild {
            session: self.session,

            established_session: Some(established_session),
            exit_status: None,

            child_stdin,
            child_stdout,
            child_stderr,
        })
    }

    /// Executes the remote command, waiting for it to finish and collecting all of its output.
    ///
    /// By default, stdout and stderr are captured (and used to provide the resulting output).
    /// Stdin is set to `Stdio::null`, and any attempt by the child process to read from
    /// the stdin stream will result in the stream immediately closing.
    pub async fn output(&mut self) -> Result<std::process::Output, Error> {
        self.spawn().await?.wait_with_output().await
    }

    /// Executes the remote command, waiting for it to finish and collecting its exit status.
    ///
    /// By default, stdin is empty, and stdout and stderr are discarded.
    pub async fn status(&mut self) -> Result<std::process::ExitStatus, Error> {
        self.spawn().await?.wait().await
    }
}
