use super::Error;
use super::RemoteChild;

use std::ffi::OsStr;
use std::process::Stdio;
use tokio::process;

#[derive(Debug)]
pub(crate) struct Command {
    builder: process::Command,
}

impl Command {
    pub(crate) fn new(mut builder: process::Command) -> Self {
        builder
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        Self { builder }
    }
}

impl Command {
    pub(crate) fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.builder.arg(arg);
        self
    }

    pub(crate) fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdin(cfg);
        self
    }

    pub(crate) fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdout(cfg);
        self
    }

    pub(crate) fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stderr(cfg);
        self
    }

    pub(crate) async fn spawn(&mut self) -> Result<RemoteChild, Error> {
        let child = self.builder.spawn().map_err(Error::Ssh)?;

        Ok(RemoteChild::new(child))
    }
}
