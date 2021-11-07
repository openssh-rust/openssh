use super::{ChildStderr, ChildStdin, ChildStdout, Error};

use core::mem::replace;

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use openssh_mux_client::connection::{EstablishedSession, SessionStatus, UNEXPECTEDEOF};

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
        use RemoteChildState::*;

        match self.state {
            Intermediate => unreachable!(),
            Exited(_exit_status) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid argument: can't kill an exited process",
            )),
            Running(_established_session) => Ok(()),
        }
    }

    pub async fn wait(&mut self) -> Result<ExitStatus, Error> {
        use RemoteChildState::*;

        let exit_value = match self.state.take() {
            Intermediate => unreachable!(),
            Exited(exit_value) => exit_value,
            Running(established_session) => match established_session.wait().await {
                Ok(session_status) => match session_status {
                    SessionStatus::TtyAllocFail(_established_session) => unreachable!(
                        "openssh::mux_client_impl does not use feature tty by any means"
                    ),
                    SessionStatus::Exited { exit_value } => exit_value,
                },
                Err((err, established_session)) => {
                    self.state = Running(established_session);
                    return Err(err.into());
                }
            },
        };

        self.state = Exited(exit_value);

        let exit_status: ExitStatus = ExitStatusExt::from_raw((exit_value as i32) << 8);

        if exit_value == UNEXPECTEDEOF {
            Err(Error::RemoteProcessTerminated)
        } else if let Some(127) = exit_status.code() {
            Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                "remote command not found",
            )))
        } else {
            Ok(exit_status)
        }
    }

    pub async fn try_wait(&mut self) -> Result<Option<ExitStatus>, Error> {
        self.wait().await.map(|val| Some(val))
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
    Exited(u32),

    /// Intermediate state means the function wait is being called.
    Intermediate,
}
impl RemoteChildState {
    fn take(&mut self) -> Self {
        replace(self, RemoteChildState::Intermediate)
    }
}
