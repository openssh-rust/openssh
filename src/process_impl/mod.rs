use std::ffi::OsStr;
use std::io;
use tokio::io::AsyncReadExt;
use tokio::process;

use super::Error;

use super::{ForwardType, Socket};

pub(crate) mod builder;

mod command;
pub(crate) use command::Command;

mod child;
pub(crate) use child::RemoteChild;

#[derive(Debug)]
pub(crate) struct Session {
    ctl: tempfile::TempDir,
    addr: String,
    terminated: bool,
    master: std::sync::Mutex<Option<(tokio::process::ChildStdout, tokio::process::ChildStderr)>>,
}

impl Session {
    fn ctl_path(&self) -> std::path::PathBuf {
        self.ctl.path().join("master")
    }

    fn new_cmd(&self, args: &[&str]) -> process::Command {
        let mut cmd = process::Command::new("ssh");
        cmd.arg("-S")
            .arg(self.ctl_path())
            .arg("-o")
            .arg("BatchMode=yes")
            .args(args)
            .arg(&self.addr);
        cmd
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        if self.terminated {
            return Err(Error::Disconnected);
        }

        let check = self
            .new_cmd(&["-O", "check"])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if let Some(255) = check.status.code() {
            if let Some(master_error) = self.take_master_error().await {
                Err(master_error)
            } else {
                Err(Error::Disconnected)
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command {
        // XXX: Should we do a self.check() here first?

        // NOTE: we pass -p 9 nine here (the "discard" port) to ensure that ssh does not
        // succeed in establishing a _new_ connection if the master connection has failed.

        let mut cmd = self.new_cmd(&["-T", "-p", "9"]);
        cmd.arg("--").arg(program);

        Command::new(cmd)
    }

    pub(crate) async fn request_port_forward(
        &self,
        forward_type: ForwardType,
        listen_socket: &Socket<'_>,
        connect_socket: &Socket<'_>,
    ) -> Result<(), Error> {
        let flag = match forward_type {
            ForwardType::Local => "-L",
            ForwardType::Remote => "-R",
        };

        let port_forwarding = self
            .new_cmd(&[
                "-fNT",
                flag,
                &format!("{}:{}", listen_socket, connect_socket),
            ])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if let Some(master_error) = self.take_master_error().await {
            return Err(master_error);
        }

        if port_forwarding.status.success() {
            Ok(())
        } else {
            let exit_err = String::from_utf8_lossy(&port_forwarding.stderr);
            let err = exit_err.trim();

            Err(Error::Ssh(io::Error::new(io::ErrorKind::Other, err)))
        }
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        if !self.terminated {
            let exit = self
                .new_terminate_cmd()
                .output()
                .await
                .map_err(Error::Ssh)?;

            self.terminated = true;

            if let Some(master_error) = self.take_master_error().await {
                return Err(master_error);
            }

            if exit.status.success() {
                return Ok(());
            }

            // let's get this case straight:
            // we tried to tell the master to exit.
            // the -O exit command failed.
            // the master exited, but did not produce an error.
            // what could cause that?
            //
            // the only thing I can think of at the moment is that the remote end cleanly
            // closed the connection, probably by virtue of being killed (but without the
            // network dropping out). since we were told to _close_ the connection, well, we
            // have succeeded, so this should not produce an error.
            //
            // we will still _collect_ the error that -O exit produced though,
            // just for ease of debugging.

            let _exit_err = String::from_utf8_lossy(&exit.stderr);
            let _err = _exit_err.trim();
            // eprintln!("{}", _err);
        }

        Ok(())
    }

    async fn take_master_error(&self) -> Option<Error> {
        let (_stdout, mut stderr) = self.master.lock().unwrap().take()?;

        let mut err = String::new();
        if let Err(e) = stderr.read_to_string(&mut err).await {
            return Some(Error::Master(e));
        }
        let stderr = err.trim();

        if stderr.is_empty() {
            return None;
        }

        let kind = if stderr.contains("Connection to") && stderr.contains("closed by remote host") {
            io::ErrorKind::ConnectionAborted
        } else {
            io::ErrorKind::Other
        };

        Some(Error::Master(io::Error::new(kind, stderr)))
    }

    fn new_terminate_cmd(&self) -> process::Command {
        self.new_cmd(&["-o", "exit"])
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self
                .new_terminate_cmd()
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}
