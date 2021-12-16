use super::Error;
use super::Stdio;

use crate::stdio::StdioImpl;

use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;

use tokio_pipe::{pipe, PipeRead, PipeWrite};

fn dup(file: &File) -> Result<File, Error> {
    file.try_clone().map_err(Error::ChildIo)
}

fn create_pipe() -> Result<(PipeRead, PipeWrite), Error> {
    pipe().map_err(Error::ChildIo)
}

/// Open "/dev/null" with RW.
fn get_null_fd() -> Result<RawFd, Error> {
    static NULL_FD: OnceCell<File> = OnceCell::new();
    let res = NULL_FD.get_or_try_init(|| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map_err(Error::ChildIo)
    });

    res.map(AsRawFd::as_raw_fd)
}

pub(crate) fn as_raw_fd(fd: &Option<File>) -> Result<RawFd, Error> {
    match fd {
        Some(fd) => Ok(AsRawFd::as_raw_fd(fd)),
        None => get_null_fd(),
    }
}

impl Stdio {
    pub(crate) fn get_stdin(&self) -> Result<(Option<File>, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((
                    // safety: read is guaranteed to contain a valid fd
                    // when `create_pipe()` succeeded.
                    Some(unsafe { File::from_raw_fd(read.into_raw_fd()) }),
                    Some(write),
                ))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn get_stdout(&self) -> Result<(Option<File>, Option<ChildStdout>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((
                    // safety: write is guaranteed to contain a valid fd
                    // when `create_pipe()` succeeded.
                    Some(unsafe { File::from_raw_fd(write.into_raw_fd()) }),
                    Some(read),
                ))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn get_stderr(&self) -> Result<(Option<File>, Option<ChildStderr>), Error> {
        self.get_stdout()
    }
}

pub(crate) type ChildStdin = PipeWrite;
pub(crate) type ChildStdout = PipeRead;
pub(crate) type ChildStderr = PipeRead;
