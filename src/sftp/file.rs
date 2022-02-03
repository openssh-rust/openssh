use super::{
    Auxiliary, Buffer, Data, Error, FileType, Id, IdCacher, Permissions, Sftp, UnixTimeStamp,
    WriteEnd,
};

use std::borrow::Cow;
use std::cmp::{min, Ordering};
use std::collections::VecDeque;
use std::convert::TryInto;
use std::future::Future;
use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::mem;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

use openssh_sftp_client::{
    AwaitableDataFuture, AwaitableStatusFuture, CreateFlags, Error as SftpError, FileAttrs,
    HandleOwned,
};

fn sftp_to_io_error(sftp_err: SftpError) -> io::Error {
    match sftp_err {
        SftpError::IOError(io_error) => io_error,
        sftp_err => io::Error::new(io::ErrorKind::Other, sftp_err),
    }
}

macro_rules! ready {
    ($e:expr) => {
        match $e {
            Poll::Ready(t) => t,
            Poll::Pending => return Poll::Pending,
        }
    };
}

/// Options and flags which can be used to configure how a file is opened.
#[derive(Debug)]
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

            buffer: Vec::new(),
            offset: 0,
            need_flush: false,

            read_future: None,
            write_futures: VecDeque::new(),
        })
    }
}

/// A reference to the remote file.
#[derive(Debug)]
pub struct File<'s> {
    phantom_data: PhantomData<&'s Sftp<'s>>,

    write_end: WriteEnd,
    handle: Arc<HandleOwned>,
    id: Option<Id>,

    is_readable: bool,
    is_writable: bool,
    need_flush: bool,

    buffer: Vec<u8>,
    offset: u64,

    read_future: Option<AwaitableDataFuture<Buffer>>,
    write_futures: VecDeque<AwaitableStatusFuture<Buffer>>,
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

    fn max_write_len(&self) -> usize {
        min(self.get_auxiliary().limits.write_len, usize::MAX as u64) as usize
    }

    fn max_read_len(&self) -> usize {
        min(self.get_auxiliary().limits.read_len, usize::MAX as u64) as usize
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
        if !self.is_writable {
            return Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            ))
            .into());
        }

        let id = self.get_id_mut();

        let mut attrs = FileAttrs::new();
        attrs.set_size(size);

        let id = self
            .write_end
            .send_fsetstat_request(id, Cow::Borrowed(&self.handle), attrs)?
            .wait()
            .await?
            .0;

        self.cache_id_mut(id);

        Ok(())
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

        if !self.is_writable {
            return Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            ))
            .into());
        }

        let id = self.get_id_mut();

        let id = self
            .write_end
            .send_fsync_request(id, Cow::Borrowed(&self.handle))?
            .wait()
            .await?
            .0;

        self.cache_id_mut(id);

        Ok(())
    }

    /// Changes the permissions on the underlying file.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_permissions(&mut self, perm: Permissions) -> Result<(), Error> {
        if !self.is_writable {
            return Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            ))
            .into());
        }

        let id = self.get_id_mut();

        let mut attrs = FileAttrs::new();
        attrs.set_permissions(perm);

        let id = self
            .write_end
            .send_fsetstat_request(id, Cow::Borrowed(&self.handle), attrs)?
            .wait()
            .await?
            .0;

        self.cache_id_mut(id);

        Ok(())
    }

    /// Queries metadata about the underlying file.
    pub async fn metadata(&mut self) -> Result<MetaData, Error> {
        if !self.is_readable {
            return Err(SftpError::from(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            ))
            .into());
        }

        let id = self.get_id_mut();

        let (id, attrs) = self
            .write_end
            .send_fstat_request(id, Cow::Borrowed(&self.handle))?
            .wait()
            .await?;

        self.cache_id_mut(id);

        Ok(MetaData(attrs))
    }

    /// Creates a new [`File`] instance that shares the same underlying
    /// file handle as the existing File instance.
    ///
    /// Reads, writes, and seeks can be performed independently.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(self.clone())
    }
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

            buffer: Vec::new(),
            offset: self.offset,
            need_flush: false,

            read_future: None,
            write_futures: VecDeque::new(),
        }
    }
}

impl AsyncSeek for File<'_> {
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

        // Reset future since they are invalidated by change of offset.
        self.read_future = None;

        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.offset))
    }
}

impl AsyncRead for File<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.is_readable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            )));
        }

        let remaining = read_buf.remaining();
        if remaining == 0 {
            return Poll::Ready(Ok(()));
        }

        let remaining = min(remaining, self.max_read_len());

        let future = if let Some(future) = &mut self.read_future {
            // Get the active future.
            //
            // The future might read more/less than remaining,
            // but the offset must be equal to self.offset,
            // since AsyncSeek::start_seek would reset self.future
            // if self.offset is changed.
            future
        } else {
            // Dereference it here once so that there will be only
            // one mutable borrow to self.
            let this = &mut *self;

            // Get id, buffer and offset to avoid reference to this.
            let id = this.get_id_mut();
            let buffer = mem::take(&mut this.buffer);
            let offset = this.offset;

            // Reference it here to make it clear that we are
            // using different part of Self.
            let write_end = &mut this.write_end;
            let handle = &this.handle;

            // Start the future
            let future = write_end
                .send_read_request(
                    id,
                    Cow::Borrowed(handle),
                    offset,
                    remaining.try_into().unwrap_or(u32::MAX),
                    Some(buffer),
                )
                .map_err(sftp_to_io_error)?
                .wait();

            // Store it in self.read_future
            self.read_future = Some(future);
            self.read_future
                .as_mut()
                .expect("FileFuture::Data is just assigned to self.future!")
        };

        // Wait for the future
        let (id, data) = ready!(Pin::new(future).poll(cx)).map_err(sftp_to_io_error)?;

        self.cache_id_mut(id);
        let buffer = match data {
            Data::Buffer(buffer) => {
                // since remaining != 0, all AwaitableDataFuture created
                // must at least read in one byte.
                debug_assert!(!buffer.is_empty());

                // sftp v3 can at most read in u32::MAX bytes.
                debug_assert!(buffer.len() <= self.max_read_len());

                buffer
            }
            Data::Eof => return Poll::Ready(Ok(())),
            _ => std::unreachable!("Expect Data::Buffer"),
        };

        // Filled the buffer
        let n = min(remaining, buffer.len());

        // Since remaining != 0 and buffer.len() != 0, n != 0.
        debug_assert_ne!(n, 0);

        read_buf.put_slice(&buffer[..n]);

        // Reuse the buffer
        if buffer.capacity() >= self.buffer.capacity() {
            self.buffer = buffer;
        }

        // Adjust offset and reset self.future
        Poll::Ready(self.start_seek(io::SeekFrom::Current(n.try_into().unwrap())))
    }
}

/// [`File::poll_write`] only writes data to the buffer.
///
/// [`File::poll_write`] and [`File::poll_write_vectored`] would
/// send at most one sftp request.
///
/// It is perfectly safe to buffer requests and send them in one go,
/// since sftp v3 guarantees that requests on the same file handler
/// is processed sequentially.
impl AsyncWrite for File<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            )));
        }

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // sftp v3 cannot send more than u32::MAX data at once.
        let buf = &buf[..min(buf.len(), self.max_write_len())];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        // Get id, buffer and offset to avoid reference to this.
        let id = this.get_id_mut();
        let offset = this.offset;

        // Reference it here to make it clear that we are
        // using different part of Self.
        let write_end = &mut this.write_end;
        let handle = &this.handle;

        let future = write_end
            .send_write_request_buffered(id, Cow::Borrowed(handle), offset, Cow::Borrowed(buf))
            .map_err(sftp_to_io_error)?
            .wait();

        self.write_futures.push_back(future);
        // Since a new future is pushed, flushing is again required.
        self.need_flush = true;

        let n = buf.len();

        // Adjust offset and reset self.future
        Poll::Ready(
            self.start_seek(io::SeekFrom::Current(n.try_into().unwrap()))
                .map(|_| n),
        )
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;

        if this.write_futures.is_empty() {
            return Poll::Ready(Ok(()));
        }

        // flush only if there is pending awaitable writes
        if this.need_flush {
            // WriteEnd::flush return true if flush succeeds, false if not.
            //
            // If it succeeds, then we no longer need to flush it.
            this.need_flush = !ready!(
                // Future returned by WriteEnd::flush does not contain
                // self-reference, so it can be optimized and placed
                // on stack.
                //
                // It is also cancel safe, so we don't need to store it.
                Pin::new(&mut Box::pin(this.write_end.flush())).poll(cx)
            )?;
        }

        loop {
            let res = if let Some(future) = this.write_futures.front_mut() {
                ready!(Pin::new(future).poll(cx))
            } else {
                // All futures consumed without error
                break Poll::Ready(Ok(()));
            };

            this.write_futures
                .pop_front()
                .expect("futures should have at least one elements in it");

            // propagate error and recycle id
            this.cache_id_mut(res.map_err(sftp_to_io_error)?.0);
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file does not support writing",
            )));
        }

        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let max_write_len = self.max_write_len();

        let mut end = 0;
        let mut n = 0;

        // loop 'buf
        //
        // This loop would skip empty `IoSlice`s.
        for buf in bufs {
            let cnt = n + buf.len();

            // branch '1
            if cnt > max_write_len {
                break;
            }

            n = cnt;
            end += 1;
        }

        let buf = if end < bufs.len() {
            let buf = &bufs[end];
            // In this branch, the loop 'buf terminate due to branch '1,
            // thus
            //
            //     n + buf.len() > max_write_len,
            //     buf.len() > max_write_len - n.
            //
            // And (max_write_len - n) also cannot be 0, otherwise
            // branch '1 will not be executed.
            let buf = &buf[..(max_write_len - n)];

            n = max_write_len;

            [IoSlice::new(buf)]
        } else {
            if n == 0 {
                return Poll::Ready(Ok(0));
            }

            [IoSlice::new(&[])]
        };
        let buffers = [&bufs[..end], &buf];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        // Get id, buffer and offset to avoid reference to this.
        let id = this.get_id_mut();
        let offset = this.offset;

        // Reference it here to make it clear that we are
        // using different part of Self.
        let write_end = &mut this.write_end;
        let handle = &this.handle;

        let future = write_end
            .send_write_request_buffered_vectored2(id, Cow::Borrowed(handle), offset, &buffers)
            .map_err(sftp_to_io_error)?
            .wait();

        self.write_futures.push_back(future);
        // Since a new future is pushed, flushing is again required.
        self.need_flush = true;

        // Adjust offset and reset self.future
        Poll::Ready(
            self.start_seek(io::SeekFrom::Current(n.try_into().unwrap()))
                .map(|_| n),
        )
    }

    fn is_write_vectored(&self) -> bool {
        true
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
