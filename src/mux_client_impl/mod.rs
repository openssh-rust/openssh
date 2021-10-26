#![allow(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    unreachable_pub
)]

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path;

use shell_escape::escape;
use tempfile::TempDir;

use tokio::runtime;

use openssh_mux_client::connection::Connection;
pub use openssh_mux_client::connection::{ForwardType, Socket};

use super::Error;

pub(crate) mod builder;

use super::fd::*;

mod stdio;
pub use stdio::{ChildStderr, ChildStdin, ChildStdout};

use super::Stdio;

mod command;
pub use command::Command;

mod child;
pub use child::RemoteChild;

#[derive(Debug)]
pub struct Session {
    /// TempDir will automatically removes the temporary dir on drop
    tempdir: Option<TempDir>,
}

// TODO: UserKnownHostsFile for custom known host fingerprint.
// TODO: Extract process output in Session::check(), Session::connect(), and Session::terminate().

impl Session {
    fn ctl(&self) -> path::PathBuf {
        self.tempdir.as_ref().unwrap().path().join("master")
    }

    pub fn get_ssh_log_path(&self) -> path::PathBuf {
        self.tempdir.as_ref().unwrap().path().join("log")
    }

    pub async fn check(&self) -> Result<(), Error> {
        Connection::connect(&self.ctl())
            .await?
            .send_alive_check()
            .await?;

        Ok(())
    }

    pub fn command<'a, S: Into<Cow<'a, str>>>(&self, program: S) -> Command {
        Command::new(self.ctl(), escape(program.into()).into())
    }

    pub fn raw_command<'a, S: AsRef<OsStr>>(&self, program: S) -> Command {
        let program = program.as_ref().to_string_lossy();
        Command::new(self.ctl(), program.to_string())
    }

    pub fn shell<S: AsRef<str>>(&self, command: S) -> Command {
        let mut cmd = self.command("sh");
        cmd.arg("-c").arg(command);
        cmd
    }

    pub async fn request_port_forward(
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

    pub async fn close(mut self) -> Result<(), Error> {
        // This also set self.tempdir to None so that Drop::drop would do nothing.
        let tempdir = self.tempdir.take().unwrap();

        Self::request_server_shutdown(&tempdir).await?;

        tempdir.close().map_err(Error::RemoveTempDir)?;

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

        let handle = runtime::Handle::try_current()
            .expect("Session should be dropped in the tokio runtime that created it");

        handle.spawn(async move {
            let _ = Self::request_server_shutdown(&tempdir).await;
        });
    }
}
