use super::{ChildStderr, ChildStdin, ChildStdout, Error, Session};

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

use tokio::io::AsyncReadExt;

use openssh_mux_client::connection::{EstablishedSession, SessionStatus};

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
    pub(crate) session: &'s Session,

    pub(crate) established_session: Option<EstablishedSession>,
    pub(crate) exit_status: Option<ExitStatus>,

    pub(crate) child_stdin: Option<ChildStdin>,
    pub(crate) child_stdout: Option<ChildStdout>,
    pub(crate) child_stderr: Option<ChildStderr>,
}

impl<'s> RemoteChild<'s> {
    /// Access the SSH session that this remote process was spawned from.
    pub fn session(&self) -> &'s Session {
        self.session
    }

    /// Disconnect from this given remote child process.
    ///
    /// Note that disconnecting does _not_ kill the remote process, it merely kills the local
    /// handle to that remote process.
    pub async fn disconnect(self) -> io::Result<()> {
        Ok(())
    }

    /// Waits for the remote child to exit completely, returning the status that it exited with.
    ///
    /// This function will continue to have the same return value after it has been called at least
    /// once.
    ///
    /// The stdin handle to the child process, if any, will be closed before waiting. This helps
    /// avoid deadlock: it ensures that the child does not block waiting for input from the parent,
    /// while the parent waits for the child to exit.
    pub async fn wait(&mut self) -> Result<ExitStatus, Error> {
        let established_session = match self.established_session.take() {
            Some(established_session) => established_session,
            None => return Ok(self.exit_status.unwrap()),
        };

        match established_session.wait().await {
            Ok(session_status) => match session_status {
                SessionStatus::TtyAllocFail(established_session) => {
                    self.established_session = Some(established_session);
                    Err(Error::TtyAllocFail)
                }
                SessionStatus::Exited {
                    conn: _conn,
                    exit_value,
                } => {
                    let exit_status = ExitStatusExt::from_raw(exit_value as i32);
                    self.exit_status = Some(exit_status);
                    Ok(exit_status)
                }
            },
            Err((err, established_session)) => {
                self.established_session = Some(established_session);
                Err(err.into())
            }
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
    pub async fn wait_with_output(mut self) -> Result<Output, Error> {
        let status = self.wait().await?;

        let mut output = Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };

        match self.child_stdout {
            Some(mut child_stdout) => {
                child_stdout
                    .read_to_end(&mut output.stdout)
                    .await
                    .map_err(|err| Error::IOError(err))?;
            }
            None => (),
        }

        match self.child_stderr {
            Some(mut child_stderr) => {
                child_stderr
                    .read_to_end(&mut output.stderr)
                    .await
                    .map_err(|err| Error::IOError(err))?;
            }
            None => (),
        }

        Ok(output)
    }

    /// Access the handle for reading from the remote child's standard input (stdin), if requested.
    pub fn stdin(&mut self) -> &mut Option<ChildStdin> {
        &mut self.child_stdin
    }

    /// Access the handle for reading from the remote child's standard output (stdout), if
    /// requested.
    pub fn stdout(&mut self) -> &mut Option<ChildStdout> {
        &mut self.child_stdout
    }

    /// Access the handle for reading from the remote child's standard error (stderr), if requested.
    pub fn stderr(&mut self) -> &mut Option<ChildStderr> {
        &mut self.child_stderr
    }
}
