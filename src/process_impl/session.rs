use super::{Command, Error, ForwardType, Socket};

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;

use tokio::process;

use tempfile::TempDir;

#[derive(Debug)]
pub(crate) struct Session {
    ctl: TempDir,
    addr: String,
    terminated: bool,
    master: Mutex<Option<PathBuf>>,
}

impl Session {
    pub(crate) fn new(ctl: TempDir, addr: &str) -> Self {
        let log = ctl.path().join("log");

        Self {
            ctl,
            addr: addr.into(),
            terminated: false,
            master: Mutex::new(Some(log)),
        }
    }

    fn ctl_path(&self) -> std::path::PathBuf {
        self.ctl.path().join("master")
    }

    fn new_cmd(&self, args: &[&str]) -> process::Command {
        let mut cmd = process::Command::new("ssh");
        cmd.stdin(Stdio::null())
            .arg("-S")
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
        forward_type: impl Into<ForwardType>,
        listen_socket: impl Into<Socket<'_>>,
        connect_socket: impl Into<Socket<'_>>,
    ) -> Result<(), Error> {
        let flag = match forward_type.into() {
            ForwardType::Local => "-L",
            ForwardType::Remote => "-R",
        };

        let port_forwarding = self
            .new_cmd(&[
                "-fNT",
                flag,
                &format!("{}:{}", &listen_socket.into(), &connect_socket.into()),
            ])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if port_forwarding.status.success() {
            Ok(())
        } else if let Some(master_error) = self.take_master_error().await {
            Err(master_error)
        } else {
            let exit_err = String::from_utf8_lossy(&port_forwarding.stderr);
            let err = exit_err.trim();

            Err(Error::Ssh(io::Error::new(io::ErrorKind::Other, err)))
        }
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        if !self.terminated {
            let exit = self
                .new_cmd(&["-o", "exit"])
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
        let log = self.master.lock().unwrap().take()?;

        let err = match fs::read_to_string(log) {
            Ok(err) => err,
            Err(e) => return Some(Error::Master(e)),
        };
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
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.terminated {
            let mut cmd = std::process::Command::new("ssh");

            let _ = cmd
                .arg("-S")
                .arg(self.ctl_path())
                .arg("-o")
                .arg("BatchMode=yes")
                .args(&["-o", "exit"])
                .arg(&self.addr)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
