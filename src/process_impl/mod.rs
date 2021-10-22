#![allow(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    unreachable_pub
)]

use std::borrow::Cow;
use std::ffi::OsStr;
use std::io;
use tokio::io::AsyncReadExt;
use tokio::process;

use super::{Error, Result};

pub mod builder;

mod command;
pub use command::Command;

mod child;
pub use child::RemoteChild;

#[derive(Debug)]
pub struct Session {
    ctl: tempfile::TempDir,
    addr: String,
    terminated: bool,
    master: std::sync::Mutex<Option<(tokio::process::ChildStdout, tokio::process::ChildStderr)>>,
}

impl Session {
    fn ctl_path(&self) -> std::path::PathBuf {
        self.ctl.path().join("master")
    }

    pub async fn check(&self) -> Result<(), Error> {
        if self.terminated {
            return Err(Error::Disconnected);
        }

        let check = process::Command::new("ssh")
            .arg("-S")
            .arg(self.ctl_path())
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-O")
            .arg("check")
            .arg(&self.addr)
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

    pub fn command<'a, S: Into<Cow<'a, str>>>(&self, program: S) -> Command {
        self.raw_command(&*shell_escape::unix::escape(program.into()))
    }

    pub fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command {
        // XXX: Should we do a self.check() here first?

        // NOTE: we pass -p 9 nine here (the "discard" port) to ensure that ssh does not
        // succeed in establishing a _new_ connection if the master connection has failed.

        let mut cmd = process::Command::new("ssh");
        cmd.arg("-S")
            .arg(self.ctl_path())
            .arg("-T")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-p")
            .arg("9")
            .arg(&self.addr)
            .arg("--")
            .arg(program);

        Command::new(cmd)
    }

    pub fn shell<S: AsRef<str>>(&self, command: S) -> Command {
        let mut cmd = self.command("sh");
        cmd.arg("-c").arg(command);
        cmd
    }

    pub async fn close(mut self) -> Result<(), Error> {
        self.terminate().await
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

    async fn terminate(&mut self) -> Result<(), Error> {
        if !self.terminated {
            let exit = process::Command::new("ssh")
                .arg("-S")
                .arg(self.ctl_path())
                .arg("-o")
                .arg("BatchMode=yes")
                .arg("-O")
                .arg("exit")
                .arg(&self.addr)
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
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = std::process::Command::new("ssh")
                .arg("-S")
                .arg(self.ctl_path())
                .arg("-o")
                .arg("BatchMode=yes")
                .arg("-O")
                .arg("exit")
                .arg(&self.addr)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}