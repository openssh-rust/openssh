use super::Error;

use std::io;
use tokio::process;

#[derive(Debug)]
pub(crate) struct RemoteChild {
    channel: Option<process::Child>,
}

impl RemoteChild {
    pub(crate) fn new(child: process::Child) -> Self {
        Self {
            channel: Some(child),
        }
    }

    pub(crate) async fn disconnect(mut self) -> io::Result<()> {
        if let Some(mut channel) = self.channel.take() {
            // this disconnects, but does not kill the remote process
            channel.kill().await?;
        }
        Ok(())
    }

    pub(crate) async fn wait(&mut self) -> Result<std::process::ExitStatus, Error> {
        match self.channel.as_mut().unwrap().wait().await {
            Err(e) => Err(Error::Remote(e)),
            Ok(w) => match w.code() {
                Some(255) => Err(Error::RemoteProcessTerminated),
                Some(127) => Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote command not found",
                ))),
                _ => Ok(w),
            },
        }
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, Error> {
        match self.channel.as_mut().unwrap().try_wait() {
            Err(e) => Err(Error::Remote(e)),
            Ok(None) => Ok(None),
            Ok(Some(w)) => match w.code() {
                Some(255) => Err(Error::RemoteProcessTerminated),
                Some(127) => Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote command not found",
                ))),
                _ => Ok(Some(w)),
            },
        }
    }

    pub(crate) fn stdin(&mut self) -> &mut Option<process::ChildStdin> {
        &mut self.channel.as_mut().unwrap().stdin
    }

    pub(crate) fn stdout(&mut self) -> &mut Option<process::ChildStdout> {
        &mut self.channel.as_mut().unwrap().stdout
    }

    pub(crate) fn stderr(&mut self) -> &mut Option<process::ChildStderr> {
        &mut self.channel.as_mut().unwrap().stderr
    }
}

impl Drop for RemoteChild {
    fn drop(&mut self) {
        if let Some(mut channel) = self.channel.take() {
            // this disconnects, but does not kill the remote process
            let _ = channel.kill();
        }
    }
}
