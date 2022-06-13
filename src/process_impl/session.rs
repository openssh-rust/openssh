use super::{Command, Error, ForwardType, Socket};

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::process;

use tempfile::TempDir;

#[derive(Debug)]
pub(crate) struct Session {
    tempdir: Option<TempDir>,
    ctl: Box<Path>,
    addr: Box<str>,
    master_log: Option<Box<Path>>,
}

impl Session {
    pub(crate) fn new(tempdir: TempDir, addr: &str) -> Self {
        let log = tempdir.path().join("log").into_boxed_path();
        let ctl = tempdir.path().join("master").into_boxed_path();

        Self {
            tempdir: Some(tempdir),
            ctl,
            addr: addr.into(),
            master_log: Some(log),
        }
    }

    pub(crate) fn resume(ctl: Box<Path>, master_log: Option<Box<Path>>) -> Self {
        Self {
            tempdir: None,
            ctl,
            // ssh don't care about the addr as long as we have the ctl.
            //
            // I tested this behavior on OpenSSH_8.9p1 Ubuntu-3, OpenSSL 3.0.2 15 Mar 2022
            addr: "none".into(),
            master_log,
        }
    }

    fn new_std_cmd(&self, args: &[impl AsRef<OsStr>]) -> std::process::Command {
        let mut cmd = std::process::Command::new("ssh");
        cmd.stdin(Stdio::null())
            .arg("-S")
            .arg(&*self.ctl)
            .arg("-o")
            .arg("BatchMode=yes")
            .args(args)
            .arg(&*self.addr);
        cmd
    }

    fn new_cmd(&self, args: &[impl AsRef<OsStr>]) -> process::Command {
        self.new_std_cmd(args).into()
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        let check = self
            .new_cmd(&["-O", "check"])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if let Some(255) = check.status.code() {
            if let Some(master_error) = self.discover_master_error() {
                Err(master_error)
            } else {
                Err(Error::Disconnected)
            }
        } else {
            Ok(())
        }
    }

    pub(crate) fn ctl(&self) -> &Path {
        &self.ctl
    }

    pub(crate) fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command {
        // XXX: Should we do a self.check() here first?

        // NOTE: we pass -p 9 nine here (the "discard" port) to ensure that ssh does not
        // succeed in establishing a _new_ connection if the master connection has failed.

        let mut cmd = self.new_cmd(&["-T", "-p", "9"]);
        cmd.arg("--").arg(program);

        Command::new(cmd)
    }

    pub(crate) fn subsystem<S: AsRef<OsStr>>(&self, program: S) -> Command {
        // XXX: Should we do a self.check() here first?

        // NOTE: we pass -p 9 nine here (the "discard" port) to ensure that ssh does not
        // succeed in establishing a _new_ connection if the master connection has failed.

        let mut cmd = self.new_cmd(&["-T", "-p", "9", "-s"]);
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
            ForwardType::Local => OsStr::new("-L"),
            ForwardType::Remote => OsStr::new("-R"),
        };

        let mut forwarding = listen_socket.into().as_os_str().into_owned();
        forwarding.push(":");
        forwarding.push(connect_socket.into().as_os_str());

        let port_forwarding = self
            .new_cmd(&[OsStr::new("-fNT"), flag, &*forwarding])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if port_forwarding.status.success() {
            Ok(())
        } else {
            let exit_err = String::from_utf8_lossy(&port_forwarding.stderr);
            let err = exit_err.trim();

            if err.is_empty() {
                if let Some(master_error) = self.discover_master_error() {
                    return Err(master_error);
                }
            }

            Err(Error::Ssh(io::Error::new(io::ErrorKind::Other, err)))
        }
    }

    async fn close_impl(&self) -> Result<(), Error> {
        let exit = self
            .new_cmd(&["-O", "exit"])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if let Some(master_error) = self.discover_master_error() {
            return Err(master_error);
        }

        // let's get this case straight:
        // we tried to tell the master to exit.
        // the -o exit command failed.
        // the master exited, but did not produce an error.
        // what could cause that?
        //
        // If the remote sshd process is accidentally killed, then the local
        // ssh multiplex server would exit without anything printed to the log,
        // and the -o exit command failed to connect to the multiplex server.
        //
        // Check `broken_connection` test in `tests/openssh.rs` for an example
        // of this scenario.
        if !exit.status.success() {
            let exit_err = String::from_utf8_lossy(&exit.stderr);
            let err = exit_err.trim();

            return Err(Error::Ssh(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                err,
            )));
        }

        Ok(())
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        // Take self.tempdir so that drop would do nothing
        if let Some(tempdir) = self.tempdir.take() {
            self.close_impl().await?;

            tempdir.close().map_err(Error::Cleanup)?;
        }

        Ok(())
    }

    pub(crate) fn detach(mut self) -> (Box<Path>, Option<Box<Path>>) {
        self.tempdir.take().map(TempDir::into_path);
        (self.ctl.clone(), self.master_log.take())
    }

    /// Forced termination, ignores value of `tempdir`
    pub(crate) async fn force_terminate(self) -> Result<(), Error> {
        if self.tempdir.is_some() {
            self.close().await
        } else {
            self.close_impl().await?;

            Ok(())
        }
    }

    fn discover_master_error(&self) -> Option<Error> {
        let err = match fs::read_to_string(self.master_log.as_ref()?) {
            Ok(err) => err,
            Err(e) => return Some(Error::Master(e)),
        };
        let mut stderr = err.trim();

        stderr = stderr.strip_prefix("ssh: ").unwrap_or(stderr);

        if stderr.starts_with("Warning: Permanently added ") {
            // added to hosts file -- let's ignore that message
            stderr = stderr.split_once('\n').map(|x| x.1.trim()).unwrap_or("");
        }

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
        // Keep tempdir alive until the connection is established
        let _tempdir = match self.tempdir.take() {
            Some(tempdir) => tempdir,
            // return since close must have already been called.
            None => return,
        };

        let _ = self
            .new_std_cmd(&["-O", "exit"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
