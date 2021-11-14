use std::ffi::OsStr;
use std::path;

use tempfile::TempDir;

use tokio::runtime;

use openssh_mux_client::connection::Connection;
pub(crate) use openssh_mux_client::connection::{ForwardType, Socket};

use super::Error;

pub(crate) mod builder;

mod fd;
use fd::*;

mod stdio;
pub(crate) use stdio::{ChildStderr, ChildStdin, ChildStdout};

use super::Stdio;

mod command;
pub(crate) use command::Command;

mod child;
pub(crate) use child::RemoteChild;

#[derive(Debug)]
pub(crate) struct Session {
    /// TempDir will automatically removes the temporary dir on drop
    tempdir: Option<TempDir>,
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

impl Session {
    fn ctl(&self) -> path::PathBuf {
        self.tempdir.as_ref().unwrap().path().join("master")
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        Connection::connect(&self.ctl())
            .await?
            .send_alive_check()
            .await?;

        Ok(())
    }

    pub(crate) fn raw_command<'a, S: AsRef<OsStr>>(&self, program: S) -> Command {
        let program = program.as_ref().to_string_lossy();
        Command::new(self.ctl(), program.to_string())
    }

    pub(crate) async fn request_port_forward(
        &self,
        forward_type: ForwardType,
        listen_socket: &Socket<'_>,
        connect_socket: &Socket<'_>,
    ) -> Result<(), Error> {
        Connection::connect(&self.ctl())
            .await?
            .request_port_forward(forward_type, listen_socket, connect_socket)
            .await?;

        Ok(())
    }

    async fn request_server_shutdown(tempdir: &TempDir) -> Result<(), Error> {
        Connection::connect(&tempdir.path().join("master"))
            .await?
            .request_stop_listening()
            .await?;

        Ok(())
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        // This also set self.tempdir to None so that Drop::drop would do nothing.
        let tempdir = self.tempdir.take().unwrap();

        Self::request_server_shutdown(&tempdir).await?;

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

        if let Ok(handle) = runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = Self::request_server_shutdown(&tempdir).await;
            });
        }
    }
}
