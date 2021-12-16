use super::{ChildStderr, ChildStdin, ChildStdout, Error};

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use openssh_mux_client::{EstablishedSession, SessionStatus};

#[derive(Debug)]
pub(crate) struct RemoteChild {
    established_session: EstablishedSession,
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
            established_session,
            child_stdin,
            child_stdout,
            child_stderr,
        }
    }

    pub(crate) async fn disconnect(self) -> io::Result<()> {
        Ok(())
    }

    pub(crate) async fn wait(self) -> Result<ExitStatus, Error> {
        let session_status = self
            .established_session
            .wait()
            .await
            .map_err(|(err, _established_session)| err)?;

        match session_status {
            SessionStatus::TtyAllocFail(_established_session) => {
                unreachable!("native_mux_impl never allocates a tty")
            }
            SessionStatus::Exited { exit_value } => {
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
            }
        }
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
