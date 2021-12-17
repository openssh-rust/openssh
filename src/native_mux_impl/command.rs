use super::Error;
use super::RemoteChild;
use super::{as_raw_fd_or_null_fd, Stdio};
use super::{ChildStderr, ChildStdin, ChildStdout};

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::Path;

use openssh_mux_client::{Connection, Session};

#[derive(Debug)]
pub(crate) struct Command<'s> {
    cmd: String,
    ctl: &'s Path,

    stdin_v: Stdio,
    stdout_v: Stdio,
    stderr_v: Stdio,
}

impl<'s> Command<'s> {
    pub(crate) fn new(ctl: &'s Path, cmd: String) -> Self {
        Self {
            cmd,
            ctl,

            stdin_v: Stdio::null(),
            stdout_v: Stdio::null(),
            stderr_v: Stdio::null(),
        }
    }

    pub(crate) fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) {
        self.cmd.push(' ');
        self.cmd.push_str(&arg.as_ref().to_string_lossy());
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
        let (stdin, child_stdin) = self.stdin_v.to_input()?;
        let (stdout, child_stdout) = self.stdout_v.to_output()?;
        let (stderr, child_stderr) = self.stderr_v.to_output()?;

        let stdios = [
            as_raw_fd_or_null_fd(&stdin)?,
            as_raw_fd_or_null_fd(&stdout)?,
            as_raw_fd_or_null_fd(&stderr)?,
        ];

        let session = Session::builder().cmd(Cow::Borrowed(&self.cmd)).build();

        let established_session = Connection::connect(self.ctl)
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
