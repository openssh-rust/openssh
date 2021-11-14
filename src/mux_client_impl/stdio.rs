use super::Error;
use super::{into_fd, Stdio};

use std::fs::File;

use tokio_pipe::{pipe, PipeRead, PipeWrite};

use crate::stdio::StdioImpl;

fn dup(file: &File) -> Result<File, Error> {
    file.try_clone().map_err(Error::ChildIo)
}

fn create_pipe() -> Result<(PipeRead, PipeWrite), Error> {
    pipe().map_err(Error::ChildIo)
}

impl Stdio {
    pub(crate) fn into_stdin(&self) -> Result<(Option<File>, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(into_fd(read)), Some(write)))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn into_stdout(&self) -> Result<(Option<File>, Option<ChildStdout>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(into_fd(write)), Some(read)))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn into_stderr(&self) -> Result<(Option<File>, Option<ChildStderr>), Error> {
        let (fd, stdout) = self.into_stdout()?;
        Ok((fd, stdout))
    }
}

pub(crate) type ChildStdin = PipeWrite;
pub(crate) type ChildStdout = PipeRead;
pub(crate) type ChildStderr = PipeRead;
