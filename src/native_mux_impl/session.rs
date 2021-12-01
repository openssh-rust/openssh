use super::{Command, Error, ForwardType, Socket};

use std::ffi::OsStr;
use std::path::PathBuf;

use openssh_mux_client::{shutdown_mux_master, Connection};
use tempfile::TempDir;

#[derive(Debug)]
pub(crate) struct Session {
    /// TempDir will automatically removes the temporary dir on drop
    tempdir: Option<TempDir>,
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

impl Session {
    pub(crate) fn new(dir: TempDir) -> Self {
        Self { tempdir: Some(dir) }
    }

    fn ctl(&self) -> PathBuf {
        self.tempdir.as_ref().unwrap().path().join("master")
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        Connection::connect(&self.ctl())
            .await?
            .send_alive_check()
            .await?;

        Ok(())
    }

    pub(crate) fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command {
        let program = program.as_ref().to_string_lossy();
        Command::new(self.ctl(), program.to_string())
    }

    pub(crate) async fn request_port_forward(
        &self,
        forward_type: impl Into<ForwardType>,
        listen_socket: impl Into<Socket<'_>>,
        connect_socket: impl Into<Socket<'_>>,
    ) -> Result<(), Error> {
        Connection::connect(&self.ctl())
            .await?
            .request_port_forward(
                forward_type.into(),
                &listen_socket.into(),
                &connect_socket.into(),
            )
            .await?;

        Ok(())
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        // This also set self.tempdir to None so that Drop::drop would do nothing.
        let tempdir = self.tempdir.take().unwrap();

        Connection::connect(&tempdir.path().join("master"))
            .await?
            .request_stop_listening()
            .await?;

        tempdir.close().map_err(Error::Cleanup)?;

        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Keep tempdir alive until the connection is established
        let tempdir = match self.tempdir.take() {
            Some(tempdir) => tempdir,
            None => return,
        };

        let _ = shutdown_mux_master(&tempdir.path().join("master"));
    }
}
