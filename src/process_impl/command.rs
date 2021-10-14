use super::RemoteChild;
use super::{Error, Result};

use std::borrow::Cow;
use std::ffi::OsStr;
use std::io;
use std::process::Stdio;
use tokio::process;

#[derive(Debug)]
pub struct Command {
    builder: process::Command,
    stdin_set: bool,
    stdout_set: bool,
    stderr_set: bool,
}

impl Command {
    pub(crate) fn new(prefix: process::Command) -> Self {
        Self {
            builder: prefix,
            stdin_set: false,
            stdout_set: false,
            stderr_set: false,
        }
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

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.builder
                .arg(&*shell_escape::unix::escape(Cow::Borrowed(arg.as_ref())));
        }
        self
    }

    pub fn raw_args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.builder.args(args);
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdin(cfg);
        self.stdin_set = true;
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stdout(cfg);
        self.stdout_set = true;
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Self {
        self.builder.stderr(cfg);
        self.stderr_set = true;
        self
    }

    pub fn spawn(&mut self) -> Result<RemoteChild> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::null());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::null());
        }
        // Then launch!
        let child = self.builder.spawn().map_err(Error::Ssh)?;

        Ok(RemoteChild::new(child))
    }

    pub async fn output(&mut self) -> Result<std::process::Output> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::piped());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::piped());
        }
        // Then launch!
        let output = self.builder.output().await.map_err(Error::Ssh)?;
        match output.status.code() {
            Some(255) => Err(Error::Disconnected),
            Some(127) => Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                &*String::from_utf8_lossy(&output.stderr),
            ))),
            _ => Ok(output),
        }
    }

    pub async fn status(&mut self) -> Result<std::process::ExitStatus> {
        // Make defaults match our defaults.
        if !self.stdin_set {
            self.builder.stdin(Stdio::null());
        }
        if !self.stdout_set {
            self.builder.stdout(Stdio::null());
        }
        if !self.stderr_set {
            self.builder.stderr(Stdio::null());
        }
        // Then launch!
        let status = self.builder.status().await.map_err(Error::Ssh)?;
        match status.code() {
            Some(255) => Err(Error::Disconnected),
            Some(127) => Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                "remote command not found",
            ))),
            _ => Ok(status),
        }
    }
}
