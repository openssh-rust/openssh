use crate::fd::Fd;

use super::Error;
use super::Stdio;

use crate::stdio::StdioImpl;

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use once_cell::sync::OnceCell;

use tokio_pipe::{pipe, PipeRead, PipeWrite};

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

pub(crate) fn as_raw_fd_or_null_fd(fd: &Option<Fd>) -> Result<RawFd, Error> {
    match fd {
        Some(fd) => Ok(AsRawFd::as_raw_fd(fd)),
        None => get_null_fd(),
    }
}

impl Stdio {
    pub(crate) fn to_input(&self) -> Result<(Option<Fd>, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(input_to_fd(read)), Some(write)))
            }
            StdioImpl::Fd(fd) => {
                if fd.get_access_mode()?.is_readable() {
                    Ok((Some(fd.try_clone()?), None))
                } else {
                    Err(Error::ChildIo(io::Error::new(
                        io::ErrorKind::Other,
                        "Fd stored in Stdio isn't readable",
                    )))
                }
            }
        }
    }

    pub(crate) fn to_output(&self) -> Result<(Option<Fd>, Option<PipeRead>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(output_to_fd(write)), Some(read)))
            }
            StdioImpl::Fd(fd) => {
                if fd.get_access_mode()?.is_writeable() {
                    Ok((Some(fd.try_clone()?), None))
                } else {
                    Err(Error::ChildIo(io::Error::new(
                        io::ErrorKind::Other,
                        "Fd stored in Stdio isn't writable",
                    )))
                }
            }
        }
    }
}

pub(crate) type ChildStdin = PipeWrite;
pub(crate) type ChildStdout = PipeRead;
pub(crate) type ChildStderr = PipeRead;

pub(crate) fn output_to_fd(output: PipeWrite) -> Fd {
    let raw_fd = output.into_raw_fd();

    // safety: output is guaranteed to contain a valid fd.
    unsafe { Fd::from_raw_fd(raw_fd) }
}

pub(crate) fn input_to_fd(input: PipeRead) -> Fd {
    let raw_fd = input.into_raw_fd();

    // safety: input is guaranteed to contain a valid fd.
    unsafe { Fd::from_raw_fd(raw_fd) }
}
