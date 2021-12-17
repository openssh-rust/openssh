use super::{Command, Error, ForwardType, Socket};

use std::ffi::OsStr;
use std::path::Path;

use openssh_mux_client::{shutdown_mux_master, Connection};
use tempfile::TempDir;

#[derive(Debug)]
pub(crate) struct Session {
    /// TempDir will automatically removes the temporary dir on drop
    tempdir: Option<TempDir>,
    ctl: Box<Path>,
}

impl Session {
    pub(crate) fn new(dir: TempDir) -> Self {
        let ctl = dir.path().join("master").into_boxed_path();

        Self {
            tempdir: Some(dir),
            ctl,
        }
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        Connection::connect(&self.ctl)
            .await?
            .send_alive_check()
            .await?;

        Ok(())
    }

    pub(crate) fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command<'_> {
        let program = program.as_ref().to_string_lossy();
        Command::new(&self.ctl, program.to_string())
    }

    pub(crate) async fn request_port_forward(
        &self,
        forward_type: impl Into<ForwardType>,
        listen_socket: impl Into<Socket<'_>>,
        connect_socket: impl Into<Socket<'_>>,
    ) -> Result<(), Error> {
        Connection::connect(&self.ctl)
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

        Connection::connect(&self.ctl)
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

        let res = shutdown_mux_master(&self.ctl);
        debug_assert!(res.is_ok(), "shutdown_mux_master failed: {:#?}", res);

        let res = tempdir.close();
        debug_assert!(res.is_ok(), "tempdir.close() failed: {:#?}", res);
    }
}
