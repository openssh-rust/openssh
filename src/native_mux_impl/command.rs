use super::Error;
use super::RemoteChild;
use super::{stdio::set_blocking, ChildStderr, ChildStdin, ChildStdout, Stdio};

use std::borrow::Cow;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use openssh_mux_client::{Connection, NonZeroByteSlice, Session};

#[derive(Debug)]
pub(crate) struct Command {
    cmd: Vec<u8>,
    ctl: Box<Path>,
    subsystem: bool,

    stdin_v: Stdio,
    stdout_v: Stdio,
    stderr_v: Stdio,
}

impl Command {
    pub(crate) fn new(ctl: Box<Path>, cmd: Vec<u8>, subsystem: bool) -> Self {
        Self {
            cmd,
            ctl,
            subsystem,

            stdin_v: Stdio::inherit(),
            stdout_v: Stdio::inherit(),
            stderr_v: Stdio::inherit(),
        }
    }

    pub(crate) fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) {
        self.cmd.push(b' ');
        self.cmd.extend_from_slice(arg.as_ref().as_bytes());
    }

    pub(crate) fn stdin<T: Into<Stdio>>(&mut self, cfg: T) {
        self.stdin_v = cfg.into();
    }

    pub(crate) fn stdout<T: Into<Stdio>>(&mut self, cfg: T) {
        self.stdout_v = cfg.into();
    }

    pub(crate) fn stderr<T: Into<Stdio>>(&mut self, cfg: T) {
        self.stderr_v = cfg.into();
    }

    pub(crate) async fn spawn(
        &mut self,
    ) -> Result<
        (
            RemoteChild,
            Option<ChildStdin>,
            Option<ChildStdout>,
            Option<ChildStderr>,
        ),
        Error,
    > {
        let (stdin, child_stdin) = self.stdin_v.to_stdin()?;
        let (stdout, child_stdout) = self.stdout_v.to_stdout()?;
        let (stderr, child_stderr) = self.stderr_v.to_stderr()?;

        let stdios = [
            stdin.as_raw_fd_or_null_fd()?,
            stdout.as_raw_fd_or_null_fd()?,
            stderr.as_raw_fd_or_null_fd()?,
        ];

        let is_inherited = [
            self.stdin_v.is_inherited(),
            self.stdout_v.is_inherited(),
            self.stderr_v.is_inherited(),
        ];

        stdios
            .into_iter()
            .zip(is_inherited)
            .filter_map(|(stdio, is_inherited)| if !is_inherited { Some(stdio) } else { None })
            // Note that once we do this, these file descriptors
            // (and the Fds we got them from above) should no longer be used in
            // any async context, as they'd start blocking.
            //
            // We give away the descriptors in stdios when we pass them to
            // open_new_session (which doesn't use them in an async context),
            // and the Fds are dropped when this function returns, meaning no
            // owned file descriptors we set to be blocking here can be used in
            // an async context in the future.
            .try_for_each(|stdio| set_blocking(stdio).map_err(Error::ChildIo))?;

        let cmd = NonZeroByteSlice::new(&self.cmd).ok_or(Error::InvalidCommand)?;

        let session = Session::builder()
            .cmd(Cow::Borrowed(cmd))
            .subsystem(self.subsystem)
            .build();

        let established_session = Connection::connect(&self.ctl)
            .await?
            .open_new_session(&session, &stdios)
            .await?;

        Ok((
            RemoteChild::new(established_session),
            child_stdin,
            child_stdout,
            child_stderr,
        ))
    }
}
