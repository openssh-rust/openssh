use super::{ChildStderr, ChildStdin, ChildStdout, Error, Session};

use std::io;
use std::process::{ExitStatus, Output};

#[derive(Debug)]
pub(crate) enum RemoteChildImp {
    ProcessImpl(super::process_impl::RemoteChild),

    #[cfg(feature = "native_mux")]
    NativeMuxImpl(super::native_mux_impl::RemoteChild),
}
impl From<super::process_impl::RemoteChild> for RemoteChildImp {
    fn from(imp: super::process_impl::RemoteChild) -> Self {
        RemoteChildImp::ProcessImpl(imp)
    }
}

#[cfg(feature = "native_mux")]
impl From<super::native_mux_impl::RemoteChild> for RemoteChildImp {
    fn from(imp: super::native_mux_impl::RemoteChild) -> Self {
        RemoteChildImp::NativeMuxImpl(imp)
    }
}

macro_rules! delegate {
    ($impl:expr, $var:ident, $then:block) => {{
        match $impl {
            RemoteChildImp::ProcessImpl($var) => $then,

            #[cfg(feature = "native_mux")]
            RemoteChildImp::NativeMuxImpl($var) => $then,
        }
    }};
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

#[inline(always)]
fn opt_into<T, U: From<T>>(opt: Option<T>) -> Option<U> {
    opt.map(|val| val.into())
}

impl<'s> RemoteChild<'s> {
    pub(crate) fn new(session: &'s Session, mut imp: RemoteChildImp) -> Self {
        Self {
            session,

            stdin: delegate!(&mut imp, imp, { opt_into(imp.stdin().take()) }),
            stdout: delegate!(&mut imp, imp, { opt_into(imp.stdout().take()) }),
            stderr: delegate!(&mut imp, imp, { opt_into(imp.stderr().take()) }),

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
        delegate!(self.imp, imp, { imp.disconnect().await })
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
        delegate!(&mut self.imp, imp, { imp.wait().await })
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
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, Error> {
        delegate!(&mut self.imp, imp, { imp.try_wait() })
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
            child_stdout.read_all(&mut output.stdout).await?;
        }

        if let Some(mut child_stderr) = self.stderr {
            child_stderr.read_all(&mut output.stderr).await?;
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
