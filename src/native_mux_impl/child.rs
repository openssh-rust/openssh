use super::{ChildStderr, ChildStdin, ChildStdout, Error};

use std::mem::replace;

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use openssh_mux_client::{EstablishedSession, SessionStatus};

macro_rules! do_wait {
    ($state:expr, $var:ident, $then:block) => {{
        let state = $state;

        let exit_value = match replace(state, RemoteChildState::AwaitingExit) {
            RemoteChildState::AwaitingExit => unreachable!(),
            RemoteChildState::Exited(exit_value) => exit_value,
            RemoteChildState::Running($var) => $then,
        };

        *state = RemoteChildState::Exited(exit_value);
        if let Some(val) = exit_value {
            if val == 127 {
                Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote command not found",
                )))
            } else {
                Ok(ExitStatusExt::from_raw((val as i32) << 8))
            }
        } else {
            Err(Error::RemoteProcessTerminated)
        }
    }};
}

#[derive(Debug)]
pub(crate) struct RemoteChild {
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

    pub(crate) async fn disconnect(self) -> io::Result<()> {
        Ok(())
    }

    pub(crate) async fn wait(mut self) -> Result<ExitStatus, Error> {
        do_wait!(&mut self.state, established_session, {
            match established_session.wait().await {
                Ok(session_status) => match session_status {
                    SessionStatus::TtyAllocFail(_established_session) => {
                        unreachable!("mux_client_impl never allocates a tty")
                    }
                    SessionStatus::Exited { exit_value } => exit_value,
                },
                Err((err, established_session)) => {
                    self.state = RemoteChildState::Running(established_session);
                    return Err(err.into());
                }
            }
        })
    }

    pub(crate) fn stdin(&mut self) -> &mut Option<ChildStdin> {
        &mut self.child_stdin
    }

    pub(crate) fn stdout(&mut self) -> &mut Option<ChildStdout> {
        &mut self.child_stdout
    }

    pub(crate) fn stderr(&mut self) -> &mut Option<ChildStderr> {
        &mut self.child_stderr
    }
}

#[derive(Debug)]
enum RemoteChildState {
    Running(EstablishedSession),
    Exited(Option<u32>),

    /// The function wait is being called.
    AwaitingExit,
}
