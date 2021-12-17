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
    ctl: Option<TempDir>,
    ctl_path: Box<Path>,
    addr: Box<str>,
    master_log: Box<Path>,
}

impl Session {
    pub(crate) fn new(ctl: TempDir, addr: &str) -> Self {
        let log = ctl.path().join("log").into_boxed_path();
        let ctl_path = ctl.path().join("master").into_boxed_path();

        Self {
            ctl: Some(ctl),
            ctl_path,
            addr: addr.into(),
            master_log: log,
        }
    }

    fn new_cmd(&self, args: &[&str]) -> process::Command {
        let mut cmd = process::Command::new("ssh");
        cmd.stdin(Stdio::null())
            .arg("-S")
            .arg(&*self.ctl_path)
            .arg("-o")
            .arg("BatchMode=yes")
            .args(args)
            .arg(&*self.addr);
        cmd
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        let check = self
            .new_cmd(&["-O", "check"])
            .output()
            .await
            .map_err(Error::Ssh)?;

        if let Some(255) = check.status.code() {
            if let Some(master_error) = self.discover_master_error().await {
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
        } else {
            let exit_err = String::from_utf8_lossy(&port_forwarding.stderr);
            let err = exit_err.trim();

            if err.is_empty() {
                if let Some(master_error) = self.discover_master_error().await {
                    return Err(master_error);
                }
            }

            Err(Error::Ssh(io::Error::new(io::ErrorKind::Other, err)))
        }
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        let mut exit_cmd = self.new_cmd(&["-o", "exit"]);

        // Take self.ctl so that drop would do nothing
        let ctl = self.ctl.take().unwrap();

        let exit = exit_cmd.output().await.map_err(Error::Ssh)?;

        if let Some(master_error) = self.discover_master_error().await {
            return Err(master_error);
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

        if !exit.status.success() {
            let _exit_err = String::from_utf8_lossy(&exit.stderr);
            let _err = _exit_err.trim();
            // eprintln!("{}", _err);
        }

        ctl.close().map_err(Error::Cleanup)?;

        Ok(())
    }

    async fn discover_master_error(&self) -> Option<Error> {
        let err = match fs::read_to_string(&self.master_log) {
            Ok(err) => err,
            Err(e) => return Some(Error::Master(e)),
        };
        let mut stderr = err.trim();

        if stderr.starts_with("ssh: ") {
            stderr = &stderr["ssh: ".len()..];
        }
        if stderr.starts_with("Warning: Permanently added ") {
            // added to hosts file -- let's ignore that message
            stderr = stderr.split_once("\r\n").map(|x| x.1).unwrap_or("");
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
        let ctl = match self.ctl.take() {
            Some(ctl) => ctl,
            None => return,
        };

        let mut cmd = std::process::Command::new("ssh");

        let res = cmd
            .arg("-S")
            .arg(&*self.ctl_path)
            .arg("-o")
            .arg("BatchMode=yes")
            .args(&["-o", "exit"])
            .arg(&*self.addr)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        debug_assert!(
            res.is_ok(),
            "process_impl::Session::drop failed: {:#?}",
            res
        );

        let res = ctl.close();
        debug_assert!(res.is_ok(), "ctl.close() failed: {:#?}", res);
    }
}
