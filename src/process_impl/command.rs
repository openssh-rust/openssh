use super::Error;
use super::RemoteChild;

use std::borrow::Cow;
use std::ffi::OsStr;
use std::process::Stdio;
use tokio::process;

#[derive(Debug)]
pub struct Command {
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
    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.raw_arg(&*shell_escape::unix::escape(Cow::Borrowed(arg.as_ref())));
        self
    }

    pub fn raw_arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Self {
        self.builder.arg(arg);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdin(cfg);
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdout(cfg);
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stderr(cfg);
        self
    }

    pub async fn spawn(&mut self) -> Result<RemoteChild, Error> {
        // Then launch!
        let child = self.builder.spawn().map_err(Error::Ssh)?;

        Ok(RemoteChild::new(child))
    }
}
