use super::child::{delegate, RemoteChildImp};
use super::{ChildStdin, ChildStdout, Error, Session};

use openssh_sftp_client::highlevel;
use std::marker::PhantomData;
use std::path::Path;
use std::process::ExitStatus;

pub use highlevel::{
    CancellationToken, DirEntry, FileType, MetaData, MetaDataBuilder, Permissions, ReadDir,
    SftpOptions, UnixTimeStamp, UnixTimeStampError,
};

/// Options and flags which can be used to configure how a file is opened.
pub type OpenOptions<'s> = highlevel::OpenOptions<'s, ChildStdin>;

/// A reference to the remote file.
///
/// Cloning [`File`] instance would return a new one that shares the same
/// underlying file handle as the existing File instance, while reads, writes
/// and seeks can be performed independently.
///
/// If you want a file that implements [`tokio::io::AsyncRead`] and
/// [`tokio::io::AsyncWrite`], checkout [`TokioCompactFile`].
pub type File<'s> = highlevel::File<'s, ChildStdin>;

/// File that implements [`tokio::io::AsyncRead`], [`tokio::io::AsyncBufRead`],
/// [`tokio::io::AsyncSeek`] and [`tokio::io::AsyncWrite`], which is compatible
/// with
/// [`tokio::fs::File`](https://docs.rs/tokio/latest/tokio/fs/struct.File.html).
pub type TokioCompactFile<'s> = highlevel::TokioCompactFile<'s, ChildStdin>;

/// A struct used to perform operations on remote filesystem.
pub type Fs<'s> = highlevel::Fs<'s, ChildStdin>;

/// Remote Directory
pub type Dir<'s> = highlevel::Dir<'s, ChildStdin>;

/// Builder for new directory to create.
pub type DirBuilder<'a, 's> = highlevel::DirBuilder<'a, 's, ChildStdin>;

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    phantom_data: PhantomData<&'s Session>,
    child: RemoteChildImp,

    inner: highlevel::Sftp<ChildStdin>,
}

impl<'s> Sftp<'s> {
    pub(crate) async fn new(
        child: RemoteChildImp,
        stdin: ChildStdin,
        stdout: ChildStdout,
        options: SftpOptions,
    ) -> Result<Sftp<'s>, Error> {
        Ok(Self {
            phantom_data: PhantomData,
            child,

            inner: highlevel::Sftp::new(stdin, stdout, options).await?,
        })
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        self.inner.close().await?;

        let res: Result<ExitStatus, Error> = delegate!(self.child, child, { child.wait().await });
        let exit_status = res?;

        if !exit_status.success() {
            Err(Error::SftpError(
                openssh_sftp_client::Error::SftpServerFailure(exit_status),
            ))
        } else {
            Ok(())
        }
    }

    /// Return a new [`OpenOptions`] object.
    pub fn options(&self) -> OpenOptions<'_> {
        self.inner.options()
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist, and will truncate
    /// it if it does.
    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.inner.create(path.as_ref()).await.map_err(Into::into)
    }

    /// Attempts to open a file in read-only mode.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.inner.open(path.as_ref()).await.map_err(Into::into)
    }

    /// [`Fs`] defaults to the current working dir set by remote `sftp-server`,
    /// which usually is the home directory.
    pub fn fs(&self) -> Fs<'_> {
        self.inner.fs()
    }

    /// Triggers the flushing of the internal buffer in `flush_task`.
    ///
    /// If there are pending requests, then flushing would happen immediately.
    ///
    /// If not, then the next time a request is queued in the write buffer, it
    /// will be immediately flushed.
    pub fn trigger_flushing(&self) {
        self.inner.trigger_flushing()
    }

    /// The maximum amount of bytes that can be written in one request.
    /// Writing more than that, then your write will be split into multiple requests
    ///
    /// If [`Sftp::max_buffered_write`] is less than [`max_atomic_write_len`],
    /// then the direct write is enabled and [`Sftp::max_write_len`] must be
    /// less than [`max_atomic_write_len`].
    pub fn max_write_len(&self) -> u32 {
        self.inner.max_write_len()
    }

    /// The maximum amount of bytes that can be read in one request.
    /// Reading more than that, then your read will be split into multiple requests
    pub fn max_read_len(&self) -> u32 {
        self.inner.max_read_len()
    }

    /// Get maximum amount of bytes that [`crate::highlevel::File`] and
    /// [`crate::highlevel::TokioCompactFile`] would write in a buffered manner.
    pub fn max_buffered_write(&self) -> u32 {
        self.inner.max_buffered_write()
    }

    /// Return a cancellation token that will be cancelled if the `flush_task`
    /// or `read_task` failed is called.
    ///
    /// Cancelling this returned token has no effect on any function in this
    /// module.
    pub fn get_cancellation_token(&self) -> CancellationToken {
        self.inner.get_cancellation_token()
    }
}
