use super::{Auxiliary, Error, FileType, Id, IdCacher, Permissions, Sftp, UnixTimeStamp, WriteEnd};

use std::borrow::Cow;
use std::cmp::{min, Ordering};
use std::future::Future;
use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use openssh_sftp_client::{CreateFlags, Data, Error as SftpError, FileAttrs, Handle, HandleOwned};
use tokio::io::AsyncSeek;

use derive_destructure2::destructure;

mod tokio_compact_file;
pub use tokio_compact_file::TokioCompactFile;

mod utility;
use utility::take_io_slices;

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

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'s>, Error> {
        let filename = Cow::Borrowed(path.as_ref());

        let params = if self.create {
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
        let id = write_end.get_thread_local_cached_id();

        let awaitable = write_end.send_open_file_request(id, params)?;
        let (id, handle) = write_end
            .get_auxiliary()
            .cancel_if_task_failed(awaitable.wait())
            .await?;

        Ok(File {
            phantom_data: PhantomData,

            write_end,
            handle: Arc::new(handle),
            id: Some(id),

            is_readable: self.options.get_read(),
            is_writable: self.options.get_write(),
            need_flush: false,
            offset: 0,
        })
    }
}

/// A reference to the remote file.
#[derive(Debug, destructure)]
pub struct File<'s> {
    phantom_data: PhantomData<&'s Sftp<'s>>,

    write_end: WriteEnd,
    handle: Arc<HandleOwned>,
    id: Option<Id>,

    is_readable: bool,
    is_writable: bool,
    need_flush: bool,
    offset: u64,
}

/// Creates a new [`File`] instance that shares the same underlying
/// file handle as the existing File instance.
///
/// Reads, writes, and seeks can be performed independently.
impl Clone for File<'_> {
    fn clone(&self) -> Self {
        Self {
            phantom_data: PhantomData,

            write_end: self.write_end.clone(),
            handle: self.handle.clone(),
            id: None,

            is_readable: self.is_readable,
            is_writable: self.is_writable,
            offset: self.offset,
            need_flush: false,
        }
    }
}

impl File<'_> {
    fn get_auxiliary(&self) -> &Auxiliary {
        self.write_end.get_auxiliary()
    }

    fn get_id_mut(&mut self) -> Id {
        self.id
            .take()
            .unwrap_or_else(|| self.write_end.get_thread_local_cached_id())
    }

    fn cache_id(&self, id: Id) {
        self.write_end.cache_id(id);
    }

    fn cache_id_mut(&mut self, id: Id) {
        if self.id.is_none() {
            self.id = Some(id);
        } else {
            self.cache_id(id);
        }
    }

    /// Get maximum amount of bytes that one single write requests
    /// can write.
    pub fn max_write_len(&self) -> u32 {
        self.get_auxiliary().limits.write_len
    }

    /// Get maximum amount of bytes that one single read requests
    /// can read.
    pub fn max_read_len(&self) -> u32 {
        self.get_auxiliary().limits.read_len
    }

    async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        let id = self.get_id_mut();

        let future = f(&mut self.write_end, Cow::Borrowed(&self.handle), id)?;

        let (id, ret) = self.get_auxiliary().cancel_if_task_failed(future).await?;

        self.cache_id_mut(id);

        Ok(ret)
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
            self.send_request(f).await
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
            self.send_request(f).await
        }
    }

    /// Close the [`File`], send the close request
    /// if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(mut self) -> Result<(), Error> {
        if Arc::strong_count(&self.handle) == 1 {
            // This is the last reference to the arc

            let res = self
                .send_request(|write_end, handle, id| {
                    Ok(write_end.send_close_request(id, handle)?.wait())
                })
                .await;

            if let Some(id) = self.id.take() {
                self.cache_id(id);
            }

            // Release resources without running `File::drop`
            self.destructure();

            res
        } else {
            Ok(())
        }
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
        self.send_writable_request(|write_end, handle, id| {
            let mut attrs = FileAttrs::new();
            attrs.set_size(size);

            Ok(write_end.send_fsetstat_request(id, handle, attrs)?.wait())
        })
        .await
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
        if !self.get_auxiliary().extensions.fsync {
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
        self.send_writable_request(|write_end, handle, id| {
            let mut attrs = FileAttrs::new();
            attrs.set_permissions(perm);

            Ok(write_end.send_fsetstat_request(id, handle, attrs)?.wait())
        })
        .await
    }

    /// Queries metadata about the underlying file.
    pub async fn metadata(&mut self) -> Result<MetaData, Error> {
        self.send_readable_request(|write_end, handle, id| {
            Ok(write_end.send_fstat_request(id, handle)?.wait())
        })
        .await
        .map(MetaData)
    }

    /// * `n` - number of bytes to read in
    ///
    /// This function can read in at most [`File::max_read_len`] bytes.
    ///
    /// If the [`File`] has reached EOF or `n == 0`, then `None` is returned.
    pub async fn read(&mut self, n: u32, buffer: Vec<u8>) -> Result<Option<Vec<u8>>, Error> {
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

    /// This function can write in at most [`File::max_write_len`] bytes.
    pub async fn write(&mut self, buf: &[u8]) -> Result<(), Error> {
        if buf.is_empty() {
            return Ok(());
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

        Ok(())
    }

    /// This function can write in at most [`File::max_write_len`] bytes.
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

impl Drop for File<'_> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.handle) == 1 {
            // This is the last reference to the arc
            let id = self.get_id_mut();
            let _ = self
                .write_end
                .send_close_request(id, Cow::Borrowed(&self.handle));
        } else if let Some(id) = self.id.take() {
            self.cache_id(id);
        }
    }
}

/// Metadata information about a file.
#[derive(Debug, Clone, Copy)]
pub struct MetaData(FileAttrs);

#[allow(clippy::len_without_is_empty)]
impl MetaData {
    /// Returns the size of the file in bytes.
    ///
    /// Return `None` if the server did not return
    /// the size.
    pub fn len(&self) -> Option<u64> {
        self.0.get_size()
    }

    /// Returns the user ID of the owner.
    ///
    /// Return `None` if the server did not return
    /// the uid.
    pub fn uid(&self) -> Option<u32> {
        self.0.get_id().map(|(uid, _gid)| uid)
    }

    /// Returns the group ID of the owner.
    ///
    /// Return `None` if the server did not return
    /// the gid.
    pub fn gid(&self) -> Option<u32> {
        self.0.get_id().map(|(_uid, gid)| gid)
    }

    /// Returns the permissions.
    ///
    /// Return `None` if the server did not return
    /// the permissions.
    pub fn permissions(&self) -> Option<Permissions> {
        self.0.get_permissions()
    }

    /// Returns the file type.
    ///
    /// Return `None` if the server did not return
    /// the file type.
    pub fn file_type(&self) -> Option<FileType> {
        self.0.get_filetype()
    }

    /// Returns the last access time.
    ///
    /// Return `None` if the server did not return
    /// the last access time.
    pub fn accessed(&self) -> Option<UnixTimeStamp> {
        self.0.get_time().map(|(atime, _mtime)| atime)
    }

    /// Returns the last modification time.
    ///
    /// Return `None` if the server did not return
    /// the last modification time.
    pub fn modified(&self) -> Option<UnixTimeStamp> {
        self.0.get_time().map(|(_atime, mtime)| mtime)
    }
}
