use super::Error;

use std::io;
use std::process::ExitStatus;

use tokio::process;

#[derive(Debug)]
pub(crate) struct RemoteChild {
    channel: Option<process::Child>,
    done: bool,
}

impl RemoteChild {
    pub(crate) fn new(child: process::Child) -> Self {
        Self {
            channel: Some(child),
            done: false,
        }
    }

    pub(crate) async fn disconnect(mut self) -> io::Result<()> {
        if let Some(mut channel) = self.channel.take() {
            if !self.done {
                // this disconnects, but does not kill the remote process
                channel.kill().await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn wait(mut self) -> Result<ExitStatus, Error> {
        match self.channel.as_mut().unwrap().wait().await {
            Err(e) => Err(Error::Remote(e)),
            Ok(w) => {
                self.done = true;
                to_remote_child_exit_status(w)
            }
        }
    }

    pub(crate) fn stdin(&mut self) -> &mut Option<process::ChildStdin> {
        &mut self
            .channel
            .as_mut()
            .expect("channel is only taken when self is consumed")
            .stdin
    }

    pub(crate) fn stdout(&mut self) -> &mut Option<process::ChildStdout> {
        &mut self
            .channel
            .as_mut()
            .expect("channel is only taken when self is consumed")
            .stdout
    }

    pub(crate) fn stderr(&mut self) -> &mut Option<process::ChildStderr> {
        &mut self
            .channel
            .as_mut()
            .expect("channel is only taken when self is consumed")
            .stderr
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

fn to_remote_child_exit_status(w: ExitStatus) -> Result<ExitStatus, Error> {
    match w.code() {
        Some(255) => Err(Error::RemoteProcessTerminated),
        Some(127) => Err(Error::Remote(io::Error::new(
            io::ErrorKind::NotFound,
            "remote command not found",
        ))),
        _ => Ok(w),
    }
}
