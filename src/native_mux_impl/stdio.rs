use crate::fd::dup;
use crate::stdio::StdioImpl;
use crate::Error;
use crate::Stdio;

use once_cell::sync::OnceCell;

use io_lifetimes::OwnedFd;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use tokio_pipe::{pipe, PipeRead, PipeWrite};

fn try_clone(fd: &OwnedFd) -> Result<OwnedFd, Error> {
    // safety: self.0 is guaranteed to contain a valid fd.
    unsafe { dup(fd.as_raw_fd()) }
}

fn get_access_mode(fd: &OwnedFd) -> Result<AccessMode, Error> {
    // safety: self.0 is guaranteed to contain a valid fd.
    let res = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };

    if res == -1 {
        Err(Error::ChildIo(io::Error::last_os_error()))
    } else {
        Ok(AccessMode(res & libc::O_ACCMODE))
    }
}

#[derive(Debug, Copy, Clone)]
struct AccessMode(libc::c_int);

impl AccessMode {
    /// Return true if the fd can only be read.
    const fn is_rdonly(&self) -> bool {
        self.0 == libc::O_RDONLY
    }

    /// Return true if the fd can only be write.
    const fn is_wronly(&self) -> bool {
        self.0 == libc::O_WRONLY
    }

    /// Return true if the fd is readable and writeable.
    const fn is_rdwr(&self) -> bool {
        self.0 == libc::O_RDWR
    }

    /// Return true if the fd can be read.
    const fn is_readable(&self) -> bool {
        self.is_rdonly() || self.is_rdwr()
    }

    /// Return true if the fd can be write.
    const fn is_writeable(&self) -> bool {
        self.is_wronly() || self.is_rdwr()
    }
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

pub(crate) fn as_raw_fd_or_null_fd(fd: &Option<OwnedFd>) -> Result<RawFd, Error> {
    match fd {
        Some(fd) => Ok(AsRawFd::as_raw_fd(fd)),
        None => get_null_fd(),
    }
}

impl Stdio {
    pub(crate) fn to_input(&self) -> Result<(Option<OwnedFd>, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(input_to_fd(read)), Some(write)))
            }
            StdioImpl::Fd(fd) => {
                if get_access_mode(fd)?.is_readable() {
                    Ok((Some(try_clone(fd)?), None))
                } else {
                    Err(Error::ChildIo(io::Error::new(
                        io::ErrorKind::Other,
                        "Fd stored in Stdio isn't readable",
                    )))
                }
            }
        }
    }

    pub(crate) fn to_output(&self) -> Result<(Option<OwnedFd>, Option<PipeRead>), Error> {
        match &self.0 {
            StdioImpl::Null => Ok((None, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Some(output_to_fd(write)), Some(read)))
            }
            StdioImpl::Fd(fd) => {
                if get_access_mode(fd)?.is_writeable() {
                    Ok((Some(try_clone(fd)?), None))
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

fn output_to_fd(output: PipeWrite) -> OwnedFd {
    let raw_fd = output.into_raw_fd();

    // safety: output is guaranteed to contain a valid fd.
    unsafe { OwnedFd::from_raw_fd(raw_fd) }
}

fn input_to_fd(input: PipeRead) -> OwnedFd {
    let raw_fd = input.into_raw_fd();

    // safety: input is guaranteed to contain a valid fd.
    unsafe { OwnedFd::from_raw_fd(raw_fd) }
}
