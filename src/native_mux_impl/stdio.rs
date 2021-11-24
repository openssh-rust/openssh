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

fn into_fd<T: IntoRawFd>(val: T) -> File {
    unsafe { File::from_raw_fd(val.into_raw_fd()) }
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
                Ok((Some(into_fd(read)), Some(write)))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn get_stdout(&self) -> Result<(Option<File>, Option<ChildStdout>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(into_fd(write)), Some(read)))
            }
            StdioImpl::Fd(fd) => Ok((Some(dup(fd)?), None)),
        }
    }

    pub(crate) fn get_stderr(&self) -> Result<(Option<File>, Option<ChildStderr>), Error> {
        let (fd, stdout) = self.get_stdout()?;
        Ok((fd, stdout))
    }
}

pub(crate) type ChildStdin = PipeWrite;
pub(crate) type ChildStdout = PipeRead;
pub(crate) type ChildStderr = PipeRead;
