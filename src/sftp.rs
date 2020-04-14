use super::{Error, Session};
use std::io::{self, prelude::*};
use std::process::Stdio;

// TODO: it would _probably_ be better to actually use sftp here, since I'm pretty sure it has some
// kind of protocol optimized for binary data. but for now, this is fine.

/// A file-oriented channel to a remote host.
///
/// You likely want [`write_to`] and [`read_from`].
#[derive(Debug, Clone)]
pub struct Sftp<'s> {
    session: &'s Session,
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Read,
    Write,
    Append,
}

impl Mode {
    fn is_write(&self) -> bool {
        match *self {
            Mode::Append | Mode::Write => true,
            Mode::Read => false,
        }
    }
}

/// A handle to a remote file.
///
/// If you opened this file for reading (with [`Sftp::read_from`]), you can read from it with
/// [`std::io::Read`]. If you opened it for writing (with [`Sftp::write_to`]), you can write to it
/// with [`std::io::Write`].
///
/// Note that because we are operating against a remote host, errors may take a while to propagate.
/// For that reason, you may find that writes to, and reads from, files that do not exist, may
/// still succeed without error. You should **make sure to call [`close`]** to observe any errors
/// that may have occurred when operating on the remote file.
#[derive(Debug)]
pub struct RemoteFile<'s> {
    cat: super::RemoteChild<'s>,
    mode: Mode,
}

// TODO: something like std::fs::OpenOptions

impl<'s> Sftp<'s> {
    pub(crate) fn new(session: &'s Session) -> Self {
        Self { session }
    }

    // TODO: remove
    // TODO: exists
    // TODO: check_access (?)

    /// Open the remote file at `path` for writing.
    ///
    /// If the remote file exists, it will be truncated. If it does not, it will be created.
    ///
    /// Note that errors may not propagate until you call [`close`], including if the remote file
    /// does not exist.
    pub fn write_to(&mut self, path: impl AsRef<std::path::Path>) -> Result<RemoteFile<'s>, Error> {
        let cat = self
            .session
            .command("tee")
            .arg(path.as_ref())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(RemoteFile {
            cat,
            mode: Mode::Write,
        })
    }

    /// Open the remote file at `path` for appending.
    ///
    /// If the remote file exists, it will be appended to. If it does not, it will be created.
    ///
    /// Note that errors may not propagate until you call [`close`], including if the remote file
    /// does not exist.
    pub fn append_to(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<RemoteFile<'s>, Error> {
        let cat = self
            .session
            .command("tee")
            .arg("-a")
            .arg(path.as_ref())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(RemoteFile {
            cat,
            mode: Mode::Append,
        })
    }

    /// Open the remote file at `path` for reading.
    ///
    /// Note that errors may not propagate until you call [`close`], including if the remote file
    /// does not exist.
    pub fn read_from(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<RemoteFile<'s>, Error> {
        let cat = self
            .session
            .command("cat")
            .arg(path.as_ref())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        Ok(RemoteFile {
            cat,
            mode: Mode::Read,
        })
    }
}

impl RemoteFile<'_> {
    /// Close the handle to the remote file.
    ///
    /// If the remote file was opened for reading, this will also call [`flush`](Write::flush).
    ///
    /// When you close the remote file, any errors on the remote end will also be propagated. This
    /// means that you could see errors about remote files not existing, or disks being full, only
    /// at the time when you call `close`.
    pub fn close(mut self) -> Result<(), Error> {
        if self.mode.is_write() {
            self.flush().map_err(Error::Remote)?;
        }

        let status = self.cat.wait()?;
        if status.success() {
            return Ok(());
        }

        // let us try to cobble together a good error for the user
        let mut stderr = self
            .cat
            .stderr()
            .take()
            .expect("stderr should always be opened for remote files");
        let mut err = String::new();
        if let Err(e) = stderr.read_to_string(&mut err) {
            return Err(Error::Remote(io::Error::new(io::ErrorKind::Other, e)));
        }
        let err = err.trim();

        if let Some(1) = status.code() {
            // looking at cat's source at the time of writing:
            // https://github.com/coreutils/coreutils/blob/730876d067f24380ccec1bdd1f179a664f11aa2f/src/cat.c
            // cat always returns with EXIT_FAILURE, which is 1 on all POSIX platforms.
        } else {
            // something really weird happened.
            return Err(Error::Remote(io::Error::new(io::ErrorKind::Other, err)));
        }

        // search for "die" or EXIT_FAILURE in the cat source code:
        // https://github.com/coreutils/coreutils/blob/master/src/cat.c
        Err(Error::Remote(if self.mode.is_write() {
            if err.ends_with(": No such file or directory") {
                io::Error::new(io::ErrorKind::NotFound, err)
            } else {
                io::Error::new(io::ErrorKind::WriteZero, err)
            }
        } else {
            if err.ends_with(": No such file or directory") {
                io::Error::new(io::ErrorKind::NotFound, err)
            } else {
                io::Error::new(io::ErrorKind::UnexpectedEof, err)
            }
        }))
    }
}

impl io::Read for RemoteFile<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.mode.is_write() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "attempted to read from remote file opened for writing",
            ));
        }

        self.cat.stdout().as_mut().unwrap().read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [io::IoSliceMut<'_>]) -> io::Result<usize> {
        if self.mode.is_write() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "attempted to read from remote file opened for writing",
            ));
        }

        self.cat.stdout().as_mut().unwrap().read_vectored(bufs)
    }
}

impl io::Write for RemoteFile<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.mode.is_write() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "attempted to write to remote file opened for reading",
            ));
        }

        self.cat.stdin().as_mut().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.cat.stdin().as_mut().unwrap().flush()
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        if !self.mode.is_write() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "attempted to write to remote file opened for reading",
            ));
        }

        self.cat.stdin().as_mut().unwrap().write_vectored(bufs)
    }
}
