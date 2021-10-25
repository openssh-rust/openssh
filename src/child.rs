use super::{ChildStderr, ChildStdin, ChildStdout, Error, Result, Session};

use std::io;
use std::process::{ExitStatus, Output};

use tokio::io::AsyncReadExt;

#[derive(Debug)]
pub(crate) enum RemoteChildImp {
    ProcessImpl(super::process_impl::RemoteChild),

    #[cfg(feature = "mux_client")]
    MuxClientImpl(super::mux_client_impl::RemoteChild),
}
impl From<super::process_impl::RemoteChild> for RemoteChildImp {
    fn from(imp: super::process_impl::RemoteChild) -> Self {
        RemoteChildImp::ProcessImpl(imp)
    }
}

#[cfg(feature = "mux_client")]
impl From<super::mux_client_impl::RemoteChild> for RemoteChildImp {
    fn from(imp: super::mux_client_impl::RemoteChild) -> Self {
        RemoteChildImp::MuxClientImpl(imp)
    }
}

/// Representation of a running or exited remote child process.
///
/// This structure is used to represent and manage remote child processes. A remote child process
/// is created via the [`Command`](crate::Command) struct through [`Session::command`], which
/// configures the spawning process and can itself be constructed using a builder-style interface.
///
/// Calling [`wait`](RemoteChild::wait) (or other functions that wrap around it) will make the
/// parent process wait until the child has actually exited before continuing.
///
/// Unlike [`std::process::Child`], `RemoteChild` *does* implement [`Drop`], and will terminate the
/// local `ssh` process corresponding to the remote process when it goes out of scope. Note that
/// this does _not_ terminate the remote process. If you want to do that, you will need to kill it
/// yourself by executing a remote command like `pkill` to kill it on the remote side.
///
/// As a result, `RemoteChild` cannot expose `stdin`, `stdout`, and `stderr` as fields for
/// split-borrows like [`std::process::Child`] does. Instead, it exposes
/// [`stdin`](RemoteChild::stdin), [`stdout`](RemoteChild::stdout),
/// and [`stderr`](RemoteChild::stderr) as methods. Callers can call `.take()` to get the same
/// effect as a split borrow and use multiple streams concurrently. Note that for the streams to be
/// available,`Stdio::piped()` should be passed to the corresponding method on
/// [`Command`](crate::Command).
///
/// ```rust,no_run
/// # async fn foo() {
/// # let child: openssh::RemoteChild<'static> = unimplemented!();
/// let stdin = child.stdin().take().unwrap();
/// let stdout = child.stdout().take().unwrap();
/// tokio::io::copy(&mut stdout, &mut stdin).await;
/// # }
/// ```
#[derive(Debug)]
pub struct RemoteChild<'s> {
    session: &'s Session,
    imp: RemoteChildImp,

    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl<'s> RemoteChild<'s> {
    pub(crate) fn new(session: &'s Session, mut imp: RemoteChildImp) -> Self {
        Self {
            session,

            stdin: match &mut imp {
                RemoteChildImp::ProcessImpl(imp) => imp.stdin().take().map(|val| val.into()),

                #[cfg(feature = "mux_client")]
                RemoteChildImp::MuxClientImpl(imp) => imp.stdin().take().map(|val| val.into()),
            },
            stdout: match &mut imp {
                RemoteChildImp::ProcessImpl(imp) => imp.stdout().take().map(|val| val.into()),

                #[cfg(feature = "mux_client")]
                RemoteChildImp::MuxClientImpl(imp) => imp.stdout().take().map(|val| val.into()),
            },
            stderr: match &mut imp {
                RemoteChildImp::ProcessImpl(imp) => imp.stderr().take().map(|val| val.into()),

                #[cfg(feature = "mux_client")]
                RemoteChildImp::MuxClientImpl(imp) => imp.stderr().take().map(|val| val.into()),
            },

            imp,
        }
    }

    /// Access the SSH session that this remote process was spawned from.
    pub fn session(&self) -> &'s Session {
        self.session
    }

    /// Disconnect from this given remote child process.
    ///
    /// Note that disconnecting does _not_ kill the remote process, it merely kills the local
    /// handle to that remote process.
    pub async fn disconnect(self) -> io::Result<()> {
        match self.imp {
            RemoteChildImp::ProcessImpl(imp) => imp.disconnect().await,

            #[cfg(feature = "mux_client")]
            RemoteChildImp::MuxClientImpl(imp) => imp.disconnect().await,
        }
    }

    /// Waits for the remote child to exit completely, returning the status that it exited with.
    ///
    /// This function will continue to have the same return value after it has been called at least
    /// once.
    ///
    /// The stdin handle to the child process, if any, will be closed before waiting. This helps
    /// avoid deadlock: it ensures that the child does not block waiting for input from the parent,
    /// while the parent waits for the child to exit.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        match &mut self.imp {
            RemoteChildImp::ProcessImpl(imp) => imp.wait().await,

            #[cfg(feature = "mux_client")]
            RemoteChildImp::MuxClientImpl(imp) => imp.wait().await,
        }
    }

    /// Attempts to collect the exit status of the remote child if it has already exited.
    ///
    /// This function will not block the calling thread and will only check to see if the child
    /// process has exited or not. If the child has exited then on Unix the process ID is reaped.
    /// This function is guaranteed to repeatedly return a successful exit status so long as the
    /// child has already exited.
    ///
    /// If the child has exited, then `Ok(Some(status))` is returned. If the exit status is not
    /// available at this time then `Ok(None)` is returned. If an error occurs, then that error is
    /// returned.
    ///
    /// Note that unlike `wait`, this function will not attempt to drop stdin.
    ///
    /// Also, this function is unimplemented!() for mux_client_impl.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        match &mut self.imp {
            RemoteChildImp::ProcessImpl(imp) => imp.try_wait(),

            #[cfg(feature = "mux_client")]
            RemoteChildImp::MuxClientImpl(_imp) => unimplemented!(),
        }
    }

    /// Simultaneously waits for the remote child to exit and collect all remaining output on the
    /// stdout/stderr handles, returning an `Output` instance.
    ///
    /// The stdin handle to the child process, if any, will be closed before waiting. This helps
    /// avoid deadlock: it ensures that the child does not block waiting for input from the parent,
    /// while the parent waits for the child to exit.
    ///
    /// By default, stdin, stdout and stderr are inherited from the parent. In order to capture the
    /// output into this `Result<Output>` it is necessary to create new pipes between parent and
    /// child. Use `stdout(Stdio::piped())` or `stderr(Stdio::piped())`, respectively.
    pub async fn wait_with_output(mut self) -> Result<Output> {
        // Close stdin so that if the remote process is reading stdin,
        // it would return EOF and the remote process can exit.
        self.stdin().take();

        let status = self.wait().await?;

        let mut output = Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };

        if let Some(mut child_stdout) = self.stdout {
            child_stdout
                .read_to_end(&mut output.stdout)
                .await
                .map_err(Error::IOError)?;
        }

        if let Some(mut child_stderr) = self.stderr {
            child_stderr
                .read_to_end(&mut output.stderr)
                .await
                .map_err(Error::IOError)?;
        }

        Ok(output)
    }

    /// Access the handle for reading from the remote child's standard input (stdin), if requested.
    pub fn stdin(&mut self) -> &mut Option<ChildStdin> {
        &mut self.stdin
    }

    /// Access the handle for reading from the remote child's standard output (stdout), if
    /// requested.
    pub fn stdout(&mut self) -> &mut Option<ChildStdout> {
        &mut self.stdout
    }

    /// Access the handle for reading from the remote child's standard error (stderr), if requested.
    pub fn stderr(&mut self) -> &mut Option<ChildStderr> {
        &mut self.stderr
    }
}
