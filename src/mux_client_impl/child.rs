use super::{ChildStderr, ChildStdin, ChildStdout, Error, Result};

use core::mem::replace;

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

use tokio::io::AsyncReadExt;

use openssh_mux_client::connection::{EstablishedSession, SessionStatus};

#[derive(Debug)]
pub struct RemoteChild {
    state: RemoteChildState,
    child_stdin: Option<ChildStdin>,
    child_stdout: Option<ChildStdout>,
    child_stderr: Option<ChildStderr>,
}

impl RemoteChild {
    pub(crate) fn new(
        established_session: EstablishedSession,
        child_stdin: Option<ChildStdin>,
        child_stdout: Option<ChildStdout>,
        child_stderr: Option<ChildStderr>,
    ) -> Self {
        Self {
            state: RemoteChildState::Running(established_session),
            child_stdin,
            child_stdout,
            child_stderr,
        }
    }

    pub async fn disconnect(self) -> io::Result<()> {
        Ok(())
    }

    pub async fn wait(&mut self) -> Result<ExitStatus> {
        use RemoteChildState::*;

        let exit_status = match self.state.take() {
            Intermediate => unreachable!(),
            Exited(exit_status) => exit_status,
            Running(established_session) => match established_session.wait().await {
                Ok(session_status) => match session_status {
                    SessionStatus::TtyAllocFail(_established_session) => unreachable!(
                        "openssh::mux_client_impl does not use feature tty by any means"
                    ),
                    SessionStatus::Exited { exit_value } => {
                        ExitStatusExt::from_raw(exit_value as i32)
                    }
                },
                Err((err, established_session)) => {
                    self.state = Running(established_session);
                    return Err(err.into());
                }
            },
        };

        self.state = Exited(exit_status);
        Ok(exit_status)
    }

    pub async fn wait_with_output(mut self) -> Result<Output> {
        self.stdin().take();
        let status = self.wait().await?;

        let mut output = Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };

        if let Some(mut child_stdout) = self.child_stdout {
            child_stdout
                .read_to_end(&mut output.stdout)
                .await
                .map_err(Error::IOError)?;
        }

        if let Some(mut child_stderr) = self.child_stderr {
            child_stderr
                .read_to_end(&mut output.stderr)
                .await
                .map_err(Error::IOError)?;
        }

        Ok(output)
    }

    pub fn stdin(&mut self) -> &mut Option<ChildStdin> {
        &mut self.child_stdin
    }

    pub fn stdout(&mut self) -> &mut Option<ChildStdout> {
        &mut self.child_stdout
    }

    pub fn stderr(&mut self) -> &mut Option<ChildStderr> {
        &mut self.child_stderr
    }
}

#[derive(Debug)]
enum RemoteChildState {
    Running(EstablishedSession),
    Exited(ExitStatus),

    /// Intermediate state means the function wait is being called.
    Intermediate,
}
impl RemoteChildState {
    fn take(&mut self) -> Self {
        replace(self, RemoteChildState::Intermediate)
    }
}
