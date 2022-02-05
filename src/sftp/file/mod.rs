use super::{Auxiliary, Error, Id, MetaData, OwnedHandle, Permissions, Sftp, SftpError, WriteEnd};

use std::borrow::Cow;
use std::cmp::{min, Ordering};
use std::future::Future;
use std::io::{self, IoSlice};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Bytes, BytesMut};
use openssh_sftp_client::{CreateFlags, Data, FileAttrs, Handle};
use tokio::io::AsyncSeek;

mod tokio_compact_file;
pub use tokio_compact_file::TokioCompactFile;

mod utility;
use utility::{take_bytes, take_io_slices};

/// Options and flags which can be used to configure how a file is opened.
#[derive(Debug, Copy, Clone)]
pub struct OpenOptions<'s> {
    sftp: &'s Sftp<'s>,
    options: openssh_sftp_client::OpenOptions,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl<'s> OpenOptions<'s> {
    pub(super) fn new(sftp: &'s Sftp<'s>) -> Self {
        Self {
            sftp,
            options: openssh_sftp_client::OpenOptions::new(),
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    /// Sets the option for read access.
    ///
    /// This option, when true, will indicate that the file
    /// should be read-able if opened.
    pub fn read(&mut self, read: bool) -> &mut Self {
        self.options = self.options.read(read);
        self
    }

    /// Sets the option for write access.
    ///
    /// This option, when true, will indicate that the file
    /// should be write-able if opened.
    ///
    /// If the file already exists, any write calls on it
    /// will overwrite its contents, without truncating it.
    pub fn write(&mut self, write: bool) -> &mut Self {
        self.options = self.options.write(write);
        self
    }

    /// Sets the option for the append mode.
    ///
    /// This option, when `true`, means that writes will append
    /// to a file instead of overwriting previous contents.
    ///
    /// Note that setting `.write(true).append(true)` has
    /// the same effect as setting only `.append(true)`.
    ///
    /// For most filesystems, the operating system guarantees that
    /// all writes are atomic: no writes get mangled because
    /// another process writes at the same time.
    ///
    /// One maybe obvious note when using append-mode:
    ///
    /// make sure that all data that belongs together is written
    /// to the file in one operation.
    ///
    /// This can be done by concatenating strings before passing them to
    /// [`File::poll_write`] or [`File::poll_write_vectored`] and
    /// calling [`File::poll_flush`] when the message is complete.
    ///
    /// Note
    ///
    /// This function doesn’t create the file if it doesn’t exist.
    /// Use the [`OpenOptions::create`] method to do so.
    pub fn append(&mut self, append: bool) -> &mut Self {
        self.options = self.options.append(append);
        self
    }

    /// Sets the option for truncating a previous file.
    ///
    /// If a file is successfully opened with this option
    /// set it will truncate the file to `0` length if it already exists.
    ///
    /// Only take effect if [`OpenOptions::create`] is set to `true`.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    /// Sets the option to create a new file, or open it if it already exists.
    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    /// Sets the option to create a new file, failing if it already exists.
    ///
    /// No file is allowed to exist at the target location,
    /// also no (dangling) symlink.
    ///
    /// In this way, if the call succeeds, the file returned
    /// is guaranteed to be new.
    ///
    /// This option is useful because it is atomic.
    ///
    /// Otherwise between checking whether a file exists and
    /// creating a new one, the file may have been
    /// created by another process (a TOCTOU race condition / attack).
    ///
    /// If `.create_new(true)` is set, `.create()` and `.truncate()` are ignored.
    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    async fn open_impl(&self, path: &Path) -> Result<File<'s>, Error> {
        let filename = Cow::Borrowed(path);

        let params = if self.create || self.create_new {
            let flags = if self.create_new {
                CreateFlags::Excl
            } else if self.truncate {
                CreateFlags::Trunc
            } else {
                CreateFlags::None
            };

            self.options.create(filename, flags, FileAttrs::new())
        } else {
            self.options.open(filename)
        };

        let mut write_end = self.sftp.write_end();

        let handle = write_end
            .send_request(|write_end, id| Ok(write_end.send_open_file_request(id, params)?.wait()))
            .await?;

        Ok(File {
            inner: OwnedHandle::new(write_end, handle),

            is_readable: self.options.get_read(),
            is_writable: self.options.get_write(),
            need_flush: false,
            offset: 0,
        })
    }

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'s>, Error> {
        self.open_impl(path.as_ref()).await
    }
}

/// A reference to the remote file.
///
/// Cloning [`File`] instance would return a new one that shares the same
/// underlying file handle as the existing File instance, while reads, writes
/// and seeks can be performed independently.
#[derive(Debug, Clone)]
pub struct File<'s> {
    inner: OwnedHandle<'s>,

    is_readable: bool,
    is_writable: bool,
    need_flush: bool,
    offset: u64,
}

impl File<'_> {
    fn get_auxiliary(&self) -> &Auxiliary {
        self.inner.get_auxiliary()
    }

    fn get_inner(&mut self) -> (&mut WriteEnd, Cow<'_, Handle>) {
        (&mut self.inner.write_end, Cow::Borrowed(&self.inner.handle))
    }

    /// Get maximum amount of bytes that one single write requests
    /// can write.
    pub fn max_write_len(&self) -> u32 {
        self.get_auxiliary().limits().write_len
    }

    /// Get maximum amount of bytes that one single read requests
    /// can read.
    pub fn max_read_len(&self) -> u32 {
        self.get_auxiliary().limits().read_len
    }

    async fn send_writable_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        if !self.is_writable {
            Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            ))
            .into())
        } else {
            self.inner.send_request(f).await
        }
    }

    async fn send_readable_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        if !self.is_readable {
            Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            ))
            .into())
        } else {
            self.inner.send_request(f).await
        }
    }

    /// Close the [`File`], send the close request
    /// if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(self) -> Result<(), Error> {
        self.inner.close().await
    }

    /// Forcibly flush the write buffer.
    ///
    /// If another thread is doing flushing, then this function would return
    /// without doing anything and return `false`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn try_flush(&self) -> Result<bool, io::Error> {
        Ok(self.inner.write_end.try_flush().await?)
    }

    /// Forcibly flush the write buffer.
    ///
    /// If another thread is doing flushing, then this function would
    /// wait until it completes or cancelled the future.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn flush(&self) -> Result<(), io::Error> {
        self.inner.write_end.flush().await?;

        Ok(())
    }

    /// Change the metadata of a file or a directory.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_metadata(&mut self, metadata: MetaData) -> Result<(), Error> {
        let attrs = metadata.into_inner();

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end.send_fsetstat_request(id, handle, attrs)?.wait())
        })
        .await
    }

    /// Truncates or extends the underlying file, updating the size
    /// of this file to become size.
    ///
    /// If the size is less than the current file’s size, then the file
    /// will be shrunk.
    ///
    /// If it is greater than the current file’s size, then the file
    /// will be extended to size and have all of the intermediate data
    /// filled in with 0s.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_len(&mut self, size: u64) -> Result<(), Error> {
        let mut attrs = FileAttrs::new();
        attrs.set_size(size);

        self.set_metadata(MetaData::new(attrs)).await
    }

    /// Attempts to sync all OS-internal metadata to disk.
    ///
    /// This function will attempt to ensure that all in-core data
    /// reaches the filesystem before returning.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn sync_all(&mut self) -> Result<(), Error> {
        if !self.get_auxiliary().extensions().fsync {
            return Err(SftpError::UnsupportedExtension(&"fsync").into());
        }

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end.send_fsync_request(id, handle)?.wait())
        })
        .await
    }

    /// Changes the permissions on the underlying file.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_permissions(&mut self, perm: Permissions) -> Result<(), Error> {
        let mut attrs = FileAttrs::new();
        attrs.set_permissions(perm);

        self.set_metadata(MetaData::new(attrs)).await
    }

    /// Queries metadata about the underlying file.
    pub async fn metadata(&mut self) -> Result<MetaData, Error> {
        self.send_readable_request(|write_end, handle, id| {
            Ok(write_end.send_fstat_request(id, handle)?.wait())
        })
        .await
        .map(MetaData::new)
    }

    /// * `n` - number of bytes to read in
    ///
    /// This function can read in at most [`File::max_read_len`] bytes.
    ///
    /// If the [`File`] has reached EOF or `n == 0`, then `None` is returned.
    pub async fn read(&mut self, n: u32, buffer: BytesMut) -> Result<Option<BytesMut>, Error> {
        if n == 0 {
            return Ok(None);
        }

        let offset = self.offset;
        let n: u32 = min(n, self.max_read_len());

        let data = self
            .send_readable_request(|write_end, handle, id| {
                Ok(write_end
                    .send_read_request(id, handle, offset, n, Some(buffer))?
                    .wait())
            })
            .await?;

        let buffer = match data {
            Data::Buffer(buffer) => buffer,
            Data::Eof => return Ok(None),
            _ => std::unreachable!("Expect Data::Buffer"),
        };

        // Adjust offset
        Pin::new(self)
            .start_seek(io::SeekFrom::Current(n as i64))
            .map_err(SftpError::from)?;

        Ok(Some(buffer))
    }

    /// This function can write in at most [`File::max_write_len`] bytes,
    /// anything longer than that will be truncated.
    pub async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let offset = self.offset;

        let max_write_len = self.max_write_len();
        let n: u32 = buf
            .len()
            .try_into()
            .map(|n| min(n, max_write_len))
            .unwrap_or(max_write_len);

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let buf = &buf[..(n as usize)];

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_buffered(id, handle, offset, Cow::Borrowed(buf))?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self)
            .start_seek(io::SeekFrom::Current(n as i64))
            .map_err(SftpError::from)?;

        Ok(n as usize)
    }

    /// This function can write in at most [`File::max_write_len`] bytes,
    /// anything longer than that will be truncated.
    pub async fn write_vectorized(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize, Error> {
        if bufs.is_empty() {
            return Ok(0);
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len();

        let (n, bufs, buf) = if let Some(res) = take_io_slices(bufs, max_write_len as usize) {
            res
        } else {
            return Ok(0);
        };

        let buffers = [bufs, &buf];

        let offset = self.offset;

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_buffered_vectored2(id, handle, offset, &buffers)?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self)
            .start_seek(io::SeekFrom::Current(n.try_into().unwrap()))
            .map_err(SftpError::from)?;

        Ok(n)
    }

    /// This function can write in at most [`File::max_write_len`] bytes,
    /// anything longer than that will be truncated.
    pub async fn write_zero_copy(&mut self, bytes_slice: &[Bytes]) -> Result<usize, Error> {
        if bytes_slice.is_empty() {
            return Ok(0);
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len();

        let (n, bufs, buf) = if let Some(res) = take_bytes(bytes_slice, max_write_len as usize) {
            res
        } else {
            return Ok(0);
        };

        let buffers = [bufs, &buf];

        let offset = self.offset;

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_zero_copy2(id, handle, offset, &buffers)?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self)
            .start_seek(io::SeekFrom::Current(n.try_into().unwrap()))
            .map_err(SftpError::from)?;

        Ok(n)
    }
}

impl AsyncSeek for File<'_> {
    /// start_seek only adjust local offset since sftp protocol
    /// does not provides a seek function.
    ///
    /// Instead, offset is provided when sending read/write requests,
    /// thus errors are reported at read/write.
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        use io::SeekFrom::*;

        match position {
            Start(pos) => {
                if pos == self.offset {
                    return Ok(());
                }

                self.offset = pos;
            }
            End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Seeking from the end is unsupported",
                ));
            }
            Current(n) => match n.cmp(&0) {
                Ordering::Equal => return Ok(()),
                Ordering::Greater => {
                    self.offset =
                        self.offset
                            .checked_add(n.try_into().unwrap())
                            .ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Overflow occured during seeking",
                                )
                            })?;
                }
                Ordering::Less => {
                    self.offset = self
                        .offset
                        .checked_sub((-n).try_into().unwrap())
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Underflow occured during seeking",
                            )
                        })?;
                }
            },
        }

        Ok(())
    }

    /// This function is a no-op.
    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.offset))
    }
}
