use crate::stdio::StdioImpl;
use crate::Error;
use crate::Stdio;

use once_cell::sync::OnceCell;

use io_lifetimes::OwnedFd;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use tokio_pipe::{pipe, PipeRead, PipeWrite};

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

pub(crate) enum Fd {
    PipeReadEnd(PipeRead),
    PipeWriteEnd(PipeWrite),

    Borrowed(RawFd),
    Null,
}

impl Fd {
    pub(crate) fn as_raw_fd_or_null_fd(&self) -> Result<RawFd, Error> {
        use Fd::*;

        match self {
            PipeReadEnd(fd) => Ok(AsRawFd::as_raw_fd(fd)),
            PipeWriteEnd(fd) => Ok(AsRawFd::as_raw_fd(fd)),

            Borrowed(rawfd) => Ok(*rawfd),
            Null => get_null_fd(),
        }
    }
}

impl Stdio {
    pub(crate) fn to_stdin(&self) -> Result<(Fd, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Inherit => Ok((Fd::Borrowed(io::stdin().as_raw_fd()), None)),
            StdioImpl::Null => Ok((Fd::Null, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Fd::PipeReadEnd(read), Some(write)))
            }
            StdioImpl::Fd(fd) => {
                if get_access_mode(fd)?.is_readable() {
                    Ok((Fd::Borrowed(fd.as_raw_fd()), None))
                } else {
                    Err(Error::ChildIo(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Fd stored in Stdio isn't readable",
                    )))
                }
            }
        }
    }

    fn to_output(
        &self,
        get_inherit_rawfd: impl FnOnce() -> RawFd,
    ) -> Result<(Fd, Option<PipeRead>), Error> {
        match &self.0 {
            StdioImpl::Inherit => Ok((Fd::Borrowed(get_inherit_rawfd()), None)),
            StdioImpl::Null => Ok((Fd::Null, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;
                Ok((Fd::PipeWriteEnd(write), Some(read)))
            }
            StdioImpl::Fd(fd) => {
                if get_access_mode(fd)?.is_writeable() {
                    Ok((Fd::Borrowed(fd.as_raw_fd()), None))
                } else {
                    Err(Error::ChildIo(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Fd stored in Stdio isn't writable",
                    )))
                }
            }
        }
    }

    pub(crate) fn to_stdout(&self) -> Result<(Fd, Option<PipeRead>), Error> {
        self.to_output(|| io::stdout().as_raw_fd())
    }

    pub(crate) fn to_stderr(&self) -> Result<(Fd, Option<PipeRead>), Error> {
        self.to_output(|| io::stderr().as_raw_fd())
    }
}

pub(crate) type ChildStdin = PipeWrite;
pub(crate) type ChildStdout = PipeRead;
pub(crate) type ChildStderr = PipeRead;
