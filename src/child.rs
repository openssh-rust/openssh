use super::Session;
use std::io;
use std::process;

/// Representation of a running or exited remote child process.
///
/// This structure is used to represent and manage remote child processes. A remote child process
/// is created via the [`Command`] struct through [`Session::command`], which configures the
/// spawning process and can itself be constructed using a builder-style interface.
///
/// Unlike [`std::process::Child`], `RemoteChild` *does* implement [`Drop`], and will terminate the
/// local `ssh` process corresponding to the remote process when it goes out of scope. Note that
/// this does _not_ terminate the remote process. If you want to do that, you will need to kill it
/// yourself by executing a remote command like `pkill` to kill it on the remote side.
///
/// Calling [`wait`](RemoteChild::wait) (or other functions that wrap around it) will make the
/// parent process wait until the child has actually exited before continuing.
#[derive(Debug)]
pub struct RemoteChild<'s> {
    pub(crate) session: &'s Session,
    pub(crate) channel: Option<process::Child>,
}

impl<'s> RemoteChild<'s> {
    /// Access the SSH session that this remote process was spawned from.
    pub fn session(&self) -> &'s Session {
        self.session
    }

    /// Waits for the remote child to exit completely, returning the status that it exited with.
    ///
    /// This function will continue to have the same return value after it has been called at least
    /// once.
    ///
    /// The stdin handle to the child process, if any, will be closed before waiting. This helps
    /// avoid deadlock: it ensures that the child does not block waiting for input from the parent,
    /// while the parent waits for the child to exit.
    pub fn wait(&mut self) -> io::Result<process::ExitStatus> {
        self.channel.as_mut().unwrap().wait()
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
    pub fn try_wait(&mut self) -> io::Result<Option<process::ExitStatus>> {
        self.channel.as_mut().unwrap().try_wait()
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
    pub fn wait_with_output(mut self) -> io::Result<process::Output> {
        self.channel.take().unwrap().wait_with_output()
    }

    /// Access the handle for reading from the remote child's standard input (stdin), if requested.
    pub fn stdin(&mut self) -> Option<&mut process::ChildStdin> {
        self.channel.as_mut().unwrap().stdin.as_mut()
    }

    /// Access the handle for reading from the remote child's standard output (stdout), if
    /// requested.
    pub fn stdout(&mut self) -> Option<&mut process::ChildStdout> {
        self.channel.as_mut().unwrap().stdout.as_mut()
    }

    /// Access the handle for reading from the remote child's standard error (stderr), if requested.
    pub fn stderr(&mut self) -> Option<&mut process::ChildStderr> {
        self.channel.as_mut().unwrap().stderr.as_mut()
    }
}

impl Drop for RemoteChild<'_> {
    fn drop(&mut self) {
        if let Some(mut channel) = self.channel.take() {
            // this disconnects, but does not kill the remote process
            let _ = channel.kill();
        }
    }
}
