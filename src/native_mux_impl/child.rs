use super::Error;

use std::io;
use std::mem;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use openssh_mux_client::{EstablishedSession, SessionStatus, TryWaitSessionStatus};

#[derive(Debug)]
pub(crate) enum RemoteChild {
    Running(EstablishedSession),
    Done(Option<u32>),
    TryingWait,
}

impl RemoteChild {
    pub(crate) fn new(established_session: EstablishedSession) -> Self {
        Self::Running(established_session)
    }

    pub(crate) async fn disconnect(self) -> io::Result<()> {
        // ssh multiplex protocol does not specify any message type
        // that can be used to kill the remote process or properly shutdown
        // the connection.
        //
        // So here we just let the drop handler does its job to release
        // underlying resources such as unix stream socket and heap memory allocated,
        // the remote process is not killed.
        Ok(())
    }

    fn process_exited_session(exit_value: Option<u32>) -> Result<ExitStatus, Error> {
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

    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, Error> {
        let tmp = mem::replace(self, RemoteChild::TryingWait);

        match tmp {
            Self::Running(established_session_old) => {
                let try_wait_session_status = established_session_old.try_wait();

                match try_wait_session_status {
                    Err((err, established_session)) => {
                        *self = Self::Running(established_session);
                        Err(err)?
                    }

                    Ok(TryWaitSessionStatus::TtyAllocFail(established_session)) => {
                        *self = Self::Running(established_session);
                        unreachable!("native_mux_impl never allocates a tty")
                    }
                    Ok(TryWaitSessionStatus::Exited { exit_value }) => {
                        *self = Self::Done(exit_value);
                        Self::process_exited_session(exit_value).map(Some)
                    }
                    Ok(TryWaitSessionStatus::InProgress(established_session)) => {
                        *self = Self::Running(established_session);
                        Ok(None)
                    }
                }
            }

            Self::Done(exit_value) => Self::process_exited_session(exit_value).map(Some),
            Self::TryingWait => panic!("Re-entrant call to try_wait"),
        }
    }

    pub(crate) async fn wait(self) -> Result<ExitStatus, Error> {
        match self {
            Self::Running(established_session) => {
                let session_status = established_session
                    .wait()
                    .await
                    .map_err(|(err, _established_session)| err)?;

                match session_status {
                    SessionStatus::TtyAllocFail(_established_session) => {
                        unreachable!("native_mux_impl never allocates a tty")
                    }
                    SessionStatus::Exited { exit_value } => {
                        Self::process_exited_session(exit_value)
                    }
                }
            }
            Self::Done(exit_value) => Self::process_exited_session(exit_value),
            Self::TryingWait => panic!("Call to wait during call to try_wait"),
        }
    }
}
