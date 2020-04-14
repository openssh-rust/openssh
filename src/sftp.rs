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

/// A file access mode.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    /// Read-only access.
    Read,
    /// Write-only access.
    Write,
    /// Write-only access in append mode.
    Append,
}

impl Mode {
    fn is_write(self) -> bool {
        match self {
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

    /// Check that the given file can be opened in the given mode.
    ///
    /// Note that this function is potentially racy. The permissions on the server could change
    /// between when you check and when you start a subsequent operation, causing it to fail. It
    /// can also produce false positives, such as if you are checking whether you can write to a
    /// file on a read-only file system that you still have write permissions on.
    pub fn can(&mut self, mode: Mode, path: impl AsRef<std::path::Path>) -> Result<(), Error> {
        let path = path.as_ref();

        // okay, so, I know it's weird to use dd for this, but hear me out here.
        // we need a command that:
        //
        //  - will error if you try to read from or write to a directory
        //  - will error if you try to read from or write to a file you do not have access to
        //  - disinguishes between the two former cases
        //  - does not create the file if it does not exist
        //  - will succeed otherwise
        //
        // cat would work for read, but would read the entire file (no good)
        // stat would not tell us if we can actually read or write the file
        // touch won't work, as it also works on directories (no error)
        // ls won't work unless we want to parse its output (which we don't)
        // test won't work because it doesn't tell us _which_ test failed
        //
        // dd does everything we need!
        let mut cmd = self.session.command("dd");
        if mode.is_write() {
            cmd.arg("if=/dev/null")
                .arg(&format!("of={}", path.display()))
                .arg("bs=1")
                .arg("count=0")
                .arg("conv=nocreat,notrunc");
        } else {
            cmd.arg(&format!("if={}", path.display()))
                .arg("of=/dev/null")
                .arg("bs=1")
                .arg("count=1");
        }

        let test = cmd.output()?;
        if test.status.success() {
            return Ok(());
        }

        // we _could_ fail here because the file does not exist.
        // if this is a write, we still need to check that the parent dir is writeable.

        // let's find out _why_ it failed
        let stderr = String::from_utf8_lossy(&test.stderr);
        let stderr = stderr.trim().trim_start_matches("dd: ");
        if stderr.contains("No such file or directory") {
            if !mode.is_write() {
                // a file that does not exist cannot be read
                return Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    &*stderr,
                )));
            } else {
                // fall-through
            }
        } else if stderr.contains("Is a directory") {
            if mode.is_write() {
                return Err(Error::Remote(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    &*stderr,
                )));
            } else {
                return Err(Error::Remote(io::Error::new(
                    io::ErrorKind::Other,
                    &*stderr,
                )));
            }
        } else {
            return Err(Error::Remote(io::Error::new(
                io::ErrorKind::PermissionDenied,
                &*stderr,
            )));
        }

        // file does not exist, but a write may be able to create it
        // we need to check if the target parent is a writeable dir
        let dir = if let Some(dir) = path.parent() {
            if dir == std::path::Path::new("") {
                std::path::Path::new(".")
            } else {
                dir
            }
        } else {
            // they're trying to write to / itself?
            return Err(Error::Remote(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trying to write to root as a file",
            )));
        };

        let test = self
            .session
            .command("test")
            .arg("(")
            .arg("-d")
            .arg(dir)
            .arg(")")
            .arg("-a")
            .arg("(")
            .arg("-w")
            .arg(dir)
            .arg(")")
            .status()?;

        if test.success() {
            Ok(())
        } else if self
            .session
            .command("test")
            .arg("-e")
            .arg(path)
            .status()?
            .success()
        {
            Err(Error::Remote(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "parent directory not writeable",
            )))
        } else {
            Err(Error::Remote(io::Error::new(
                io::ErrorKind::NotFound,
                "parent directory does not exist",
            )))
        }
    }

    /// Initialize the given operation on the target file.
    ///
    /// Note that this function is not guaranteed to be side-effect free for writes. Specifically,
    /// it will create the target file in order to test whether it is writeable.
    fn init_op(&mut self, mode: Mode, path: impl AsRef<std::path::Path>) -> Result<(), Error> {
        if mode.is_write() {
            // for writes, we want a stronger (and cheaper) test than can()
            // we can't actually use `touch`, since it also works for dirs
            // note that this works b/c stdin is Stdio::null()
            let touch = self
                .session
                .command("cat")
                .arg(">>")
                .arg(path.as_ref())
                .stdin(Stdio::null())
                .output()?;
            if touch.status.success() {
                return Ok(());
            }

            // let's find out _why_ it failed
            let stderr = String::from_utf8_lossy(&touch.stderr);
            if stderr.contains("No such file or directory") {
                Err(Error::Remote(io::Error::new(
                    io::ErrorKind::NotFound,
                    &*stderr,
                )))
            } else {
                Err(Error::Remote(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    &*stderr,
                )))
            }
        } else {
            self.can(Mode::Read, path)
        }
    }

    /// Open the remote file at `path` for writing.
    ///
    /// If the remote file exists, it will be truncated. If it does not, it will be created.
    ///
    /// Note that errors may not propagate until you call [`close`], including if the remote file
    /// does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use openssh::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::io::prelude::*;
    ///
    /// // connect to a remote host and get an sftp connection
    /// let session = Session::connect("host", KnownHosts::Strict)?;
    /// let mut sftp = session.sftp();
    ///
    /// // open a file for writing
    /// let mut w = sftp.write_to("test_file")?;
    ///
    /// // write something to the file
    /// write!(w, "hello world")?;
    ///
    /// // flush and close the remote file, absorbing any final errors
    /// w.close()?;
    /// # Ok(())
    /// # }
    pub fn write_to(&mut self, path: impl AsRef<std::path::Path>) -> Result<RemoteFile<'s>, Error> {
        let path = path.as_ref();

        self.init_op(Mode::Write, path)?;

        let cat = self
            .session
            .command("tee")
            .arg(path)
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
    /// does not exist. You may wish to first call [`can(Mode::Append)`](can).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use openssh::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::io::prelude::*;
    ///
    /// // connect to a remote host and get an sftp connection
    /// let session = Session::connect("host", KnownHosts::Strict)?;
    /// let mut sftp = session.sftp();
    ///
    /// // open a file for appending
    /// let mut w = sftp.append_to("test_file")?;
    ///
    /// // write will append to the file
    /// write!(w, "hello world")?;
    ///
    /// // flush and close the remote file, absorbing any final errors
    /// w.close()?;
    /// # Ok(())
    /// # }
    pub fn append_to(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<RemoteFile<'s>, Error> {
        let path = path.as_ref();

        self.init_op(Mode::Append, path)?;

        let cat = self
            .session
            .command("tee")
            .arg("-a")
            .arg(path)
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
    /// does not exist. You may wish to first call [`can(Mode::Read)`](can).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use openssh::*;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::io::prelude::*;
    ///
    /// // connect to a remote host and get an sftp connection
    /// let session = Session::connect("host", KnownHosts::Strict)?;
    /// let mut sftp = session.sftp();
    ///
    /// // open a file for reading
    /// let mut r = sftp.read_from("/etc/hostname")?;
    ///
    /// // write something to the file
    /// let mut contents = String::new();
    /// r.read_to_string(&mut contents)?;
    ///
    /// // close the remote file, absorbing any final errors
    /// r.close()?;
    /// # Ok(())
    /// # }
    pub fn read_from(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<RemoteFile<'s>, Error> {
        let path = path.as_ref();

        self.init_op(Mode::Read, path)?;

        let cat = self
            .session
            .command("cat")
            .arg(path)
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
        #[allow(clippy::collapsible_if)]
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
