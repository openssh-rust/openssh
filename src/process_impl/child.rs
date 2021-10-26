use super::Error;

use std::io;
use tokio::process;

#[derive(Debug)]
pub struct RemoteChild {
    channel: Option<process::Child>,
}

impl RemoteChild {
    pub(crate) fn new(child: process::Child) -> Self {
        Self {
            channel: Some(child),
        }
    }

    pub async fn disconnect(mut self) -> io::Result<()> {
        if let Some(mut channel) = self.channel.take() {
            // this disconnects, but does not kill the remote process
            channel.kill().await?;
        }
        Ok(())
    }

    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, Error> {
        match self.channel.as_mut().unwrap().wait().await {
            Err(e) => Err(Error::Remote(e)),
            Ok(w) => match w.code() {
                Some(255) => Err(Error::Disconnected),
                Some(127) => Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote command not found",
                ))),
                _ => Ok(w),
            },
        }
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, Error> {
        match self.channel.as_mut().unwrap().try_wait() {
            Err(e) => Err(Error::Remote(e)),
            Ok(None) => Ok(None),
            Ok(Some(w)) => match w.code() {
                Some(255) => Err(Error::Disconnected),
                Some(127) => Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote command not found",
                ))),
                _ => Ok(Some(w)),
            },
        }
    }

    pub async fn wait_with_output(mut self) -> Result<std::process::Output, Error> {
        match self.channel.take().unwrap().wait_with_output().await {
            Err(e) => Err(Error::Remote(e)),
            Ok(w) => match w.status.code() {
                Some(255) => Err(Error::Disconnected),
                Some(127) => Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    &*String::from_utf8_lossy(&w.stderr),
                ))),
                _ => Ok(w),
            },
        }
    }

    pub fn stdin(&mut self) -> &mut Option<process::ChildStdin> {
        &mut self.channel.as_mut().unwrap().stdin
    }

    pub fn stdout(&mut self) -> &mut Option<process::ChildStdout> {
        &mut self.channel.as_mut().unwrap().stdout
    }

    pub fn stderr(&mut self) -> &mut Option<process::ChildStderr> {
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
