use crate::{stdio::StdioImpl, Error, Stdio};

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
};

use libc::{c_int, fcntl, F_GETFL, F_SETFL, O_NONBLOCK};
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

pub(crate) enum Fd {
    Owned(OwnedFd),
    Borrowed(RawFd),
    Null,
}

fn cvt(ret: c_int) -> io::Result<c_int> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

fn set_blocking_inner(fd: RawFd) -> io::Result<()> {
    let flags = cvt(unsafe { fcntl(fd, F_GETFL) })?;
    cvt(unsafe { fcntl(fd, F_SETFL, flags & (!O_NONBLOCK)) })?;

    Ok(())
}

fn set_blocking(fd: RawFd) -> Result<(), Error> {
    set_blocking_inner(fd).map_err(Error::ChildIo)
}

impl Fd {
    pub(crate) fn as_raw_fd_or_null_fd(&self) -> Result<RawFd, Error> {
        use Fd::*;

        match self {
            Owned(owned_fd) => Ok(owned_fd.as_raw_fd()),
            Borrowed(rawfd) => Ok(*rawfd),
            Null => get_null_fd(),
        }
    }

    /// # Safety
    ///
    /// `T::into_raw_fd` must return a valid fd and transfers
    /// the ownershipt of it.
    unsafe fn new_owned<T: IntoRawFd>(fd: T) -> Result<Self, Error> {
        let raw_fd = fd.into_raw_fd();
        Ok(Fd::Owned(OwnedFd::from_raw_fd(raw_fd)))
    }
}

impl TryFrom<PipeRead> for Fd {
    type Error = Error;

    fn try_from(pipe_read: PipeRead) -> Result<Self, Error> {
        // Safety:
        //
        // PipeRead::into_raw_fd returns a valid fd and transfers the
        // ownership of it.
        unsafe { Self::new_owned(pipe_read) }
    }
}

impl TryFrom<PipeWrite> for Fd {
    type Error = Error;

    fn try_from(pipe_write: PipeWrite) -> Result<Self, Error> {
        // Safety:
        //
        // PipeWrite::into_raw_fd returns a valid fd and transfers the
        // ownership of it.
        unsafe { Self::new_owned(pipe_write) }
    }
}

impl Stdio {
    pub(crate) fn to_stdin(&self) -> Result<(Fd, Option<ChildStdin>), Error> {
        match &self.0 {
            StdioImpl::Inherit => Ok((Fd::Borrowed(io::stdin().as_raw_fd()), None)),
            StdioImpl::Null => Ok((Fd::Null, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;

                // read end will be sent to ssh multiplex server
                // and it expects blocking fd.
                set_blocking(read.as_raw_fd())?;
                Ok((read.try_into()?, Some(write)))
            }
            StdioImpl::Fd(fd, owned) => {
                let raw_fd = fd.as_raw_fd();
                if *owned {
                    set_blocking(raw_fd)?;
                }
                Ok((Fd::Borrowed(raw_fd), None))
            }
        }
    }

    fn to_output(&self, get_inherit_rawfd: fn() -> RawFd) -> Result<(Fd, Option<PipeRead>), Error> {
        match &self.0 {
            StdioImpl::Inherit => Ok((Fd::Borrowed(get_inherit_rawfd()), None)),
            StdioImpl::Null => Ok((Fd::Null, None)),
            StdioImpl::Pipe => {
                let (read, write) = create_pipe()?;

                // write end will be sent to ssh multiplex server
                // and it expects blocking fd.
                set_blocking(write.as_raw_fd())?;
                Ok((write.try_into()?, Some(read)))
            }
            StdioImpl::Fd(fd, owned) => {
                let raw_fd = fd.as_raw_fd();
                if *owned {
                    set_blocking(raw_fd)?;
                }
                Ok((Fd::Borrowed(raw_fd), None))
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
