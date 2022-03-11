use super::{Command, Error, ForwardType, Socket};
use crate::builder::SessionBuilder;

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::process;

#[derive(Debug)]
pub(crate) struct Session {
    builder: SessionBuilder,
    addr: Box<str>,
}

impl Session {
    pub(crate) fn new(builder: SessionBuilder, addr: &str) -> Self {
        Self {
            builder,
            addr: addr.into(),
        }
    }

    fn new_cmd(&self, args: &[impl AsRef<OsStr>]) -> process::Command {
        let mut cmd = process::Command::new("ssh");
        self.builder.apply_options(&mut cmd);
        cmd.stdin(Stdio::null())
            .arg("-o")
            .arg("BatchMode=yes")
            .args(args)
            .arg(&*self.addr);
        cmd
    }

    pub(crate) async fn check(&self) -> Result<(), Error> {
        Ok(())
    }

    pub(crate) fn raw_command<S: AsRef<OsStr>>(&self, program: S) -> Command {
        let mut cmd = self.new_cmd(&["-T"]);
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
            Self::parse_stderr(port_forwarding.stderr)
        }
    }

    pub(crate) async fn close(mut self) -> Result<(), Error> {
        Ok(())
    }

    fn parse_stderr(&self, stderr: Vec<u8>) -> Error {
        let stderr = String::from_utf8_lossy(stderr);

        let mut stderr = stderr.trim();

        stderr = stderr.strip_prefix("ssh: ").unwrap_or(stderr);

        if stderr.starts_with("Warning: Permanently added ") {
            // added to hosts file -- let's ignore that message
            stderr = stderr.split_once('\n').map(|x| x.1.trim()).unwrap_or("");
        }

        let kind = if stderr.contains("Connection to") && stderr.contains("closed by remote host") {
            io::ErrorKind::ConnectionAborted
        } else {
            io::ErrorKind::Other
        };

        Error::Ssh(io::Error::new(kind, stderr))
    }
}
