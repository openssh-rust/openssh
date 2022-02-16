use super::super::{BoxedWaitForCancellationFuture, Buffer, Data};
use super::utility::take_io_slices;
use super::{Error, File, Id, SftpError, WriteEnd};

use std::borrow::Cow;
use std::cmp::min;
use std::collections::VecDeque;
use std::future::Future;
use std::io::{self, IoSlice};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::BytesMut;
use openssh_sftp_client::{AwaitableDataFuture, AwaitableStatusFuture, Handle};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

use derive_destructure2::destructure;

macro_rules! ready {
    ($e:expr) => {
        match $e {
            Poll::Ready(t) => t,
            Poll::Pending => return Poll::Pending,
        }
    };
}

fn sftp_to_io_error(sftp_err: SftpError) -> io::Error {
    match sftp_err {
        SftpError::IOError(io_error) => io_error,
        sftp_err => io::Error::new(io::ErrorKind::Other, sftp_err),
    }
}

fn send_request<Func, R>(file: &mut File<'_>, f: Func) -> Result<R, io::Error>
where
    Func: FnOnce(&mut WriteEnd, Id, Cow<'_, Handle>, u64) -> Result<R, SftpError>,
{
    // Get id and offset to avoid reference to file.
    let id = file.inner.get_id_mut();
    let offset = file.offset;

    let (write_end, handle) = file.get_inner();

    // Add request to write buffer
    let awaitable = f(write_end, id, handle, offset).map_err(sftp_to_io_error)?;

    // Requests is already added to write buffer, so wakeup
    // the `flush_task`.
    write_end.get_auxiliary().wakeup_flush_task();

    Ok(awaitable)
}

/// File that implements [`AsyncRead`], [`AsyncSeek`] and [`AsyncWrite`],
/// that is compatible with
/// [`tokio::fs::File`](https://docs.rs/tokio/latest/tokio/fs/struct.File.html).
#[derive(Debug, destructure)]
pub struct TokioCompactFile<'s> {
    inner: File<'s>,

    buffer: BytesMut,

    read_future: Option<AwaitableDataFuture<Buffer>>,
    read_cancellation_future: BoxedWaitForCancellationFuture<'s>,

    write_futures: VecDeque<AwaitableStatusFuture<Buffer>>,
    write_cancellation_future: BoxedWaitForCancellationFuture<'s>,
}

impl<'s> TokioCompactFile<'s> {
    /// Create a [`TokioCompactFile`].
    pub fn new(inner: File<'s>) -> Self {
        Self {
            inner,

            buffer: BytesMut::new(),

            read_future: None,
            read_cancellation_future: BoxedWaitForCancellationFuture::new(),

            write_futures: VecDeque::new(),
            write_cancellation_future: BoxedWaitForCancellationFuture::new(),
        }
    }

    /// Return the inner [`File`].
    pub fn into_inner(self) -> File<'s> {
        self.destructure().0
    }

    /// Flush the write buffer, wait for the status report and send
    /// the close request if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(mut self) -> Result<(), Error> {
        let need_flush = self.need_flush;
        let write_end = &mut self.inner.inner.write_end;

        if need_flush {
            write_end.sftp().trigger_flushing();
        }

        while let Some(future) = self.write_futures.pop_front() {
            let id = write_end.cancel_if_task_failed(future).await?.0;
            write_end.cache_id_mut(id);
        }

        self.into_inner().close().await
    }
}

impl<'s> From<File<'s>> for TokioCompactFile<'s> {
    fn from(inner: File<'s>) -> Self {
        Self::new(inner)
    }
}

impl<'s> From<TokioCompactFile<'s>> for File<'s> {
    fn from(file: TokioCompactFile<'s>) -> Self {
        file.into_inner()
    }
}

/// Creates a new [`TokioCompactFile`] instance that shares the
/// same underlying file handle as the existing File instance.
///
/// Reads, writes, and seeks can be performed independently.
impl Clone for TokioCompactFile<'_> {
    fn clone(&self) -> Self {
        let mut inner = self.inner.clone();
        inner.need_flush = false;
        Self::new(inner)
    }
}

impl<'s> Deref for TokioCompactFile<'s> {
    type Target = File<'s>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TokioCompactFile<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl AsyncSeek for TokioCompactFile<'_> {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        let prev_offset = self.offset();
        Pin::new(&mut self.inner).start_seek(position)?;
        let new_offset = self.offset();

        if new_offset != prev_offset {
            // Reset future since they are invalidated by change of offset.
            self.read_future = None;
        }

        Ok(())
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Pin::new(&mut self.inner).poll_complete(cx)
    }
}

/// [`TokioCompactFile`] can read in at most [`File::max_read_len`] bytes
/// at a time.
impl AsyncRead for TokioCompactFile<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        if !this.is_readable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            )));
        }

        let remaining = read_buf.remaining();
        if remaining == 0 {
            return Poll::Ready(Ok(()));
        }

        let remaining = min(remaining, this.max_read_len() as usize);

        let future = if let Some(future) = &mut this.read_future {
            // Get the active future.
            //
            // The future might read more/less than remaining,
            // but the offset must be equal to this.offset,
            // since AsyncSeek::start_seek would reset this.future
            // if this.offset is changed.
            future
        } else {
            this.buffer.clear();
            this.buffer.reserve(remaining);
            let cap = this.buffer.capacity();
            let buffer = this.buffer.split_off(cap - remaining);

            let future = send_request(&mut this.inner, |write_end, id, handle, offset| {
                write_end.send_read_request(
                    id,
                    handle,
                    offset,
                    remaining.try_into().unwrap_or(u32::MAX),
                    Some(buffer),
                )
            })?
            .wait();

            // Store it in this.read_future
            this.read_future = Some(future);
            this.read_future
                .as_mut()
                .expect("FileFuture::Data is just assigned to self.future!")
        };

        this.read_cancellation_future
            .poll_for_task_failure(cx, this.inner.get_auxiliary())?;

        // Wait for the future
        let (id, data) = ready!(Pin::new(future).poll(cx)).map_err(sftp_to_io_error)?;

        this.inner.inner.cache_id_mut(id);
        let buffer = match data {
            Data::Buffer(buffer) => {
                // since remaining != 0, all AwaitableDataFuture created
                // must at least read in one byte.
                debug_assert!(!buffer.is_empty());

                // sftp v3 can at most read in u32::MAX bytes.
                debug_assert!(buffer.len() <= this.max_read_len() as usize);

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

        // Adjust offset and reset this.future
        Poll::Ready(self.start_seek(io::SeekFrom::Current(n.try_into().unwrap())))
    }
}

/// [`TokioCompactFile::poll_write`] only writes data to the buffer.
///
/// [`TokioCompactFile::poll_write`] and
/// [`TokioCompactFile::poll_write_vectored`] would send at most one
/// sftp request.
///
/// It is perfectly safe to buffer requests and send them in one go,
/// since sftp v3 guarantees that requests on the same file handler
/// is processed sequentially.
///
/// NOTE that these writes cannot be cancelled.
///
/// One maybe obvious note when using append-mode:
///
/// make sure that all data that belongs together is written
/// to the file in one operation.
///
/// This can be done by concatenating strings before passing them to
/// [`AsyncWrite::poll_write`] or [`AsyncWrite::poll_write_vectored`] and
/// calling [`AsyncWrite::poll_flush`] on [`TokioCompactFile`] when the message
/// is complete.
///
/// Calling [`AsyncWrite::poll_flush`] on [`TokioCompactFile`] would wait on
/// writes in the order they are sent.
///
/// [`TokioCompactFile`] can write at most [`File::max_write_len`] bytes
/// at a time.
impl AsyncWrite for TokioCompactFile<'_> {
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

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let buf = &buf[..min(buf.len(), self.max_write_len() as usize)];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        let future = send_request(&mut this.inner, |write_end, id, handle, offset| {
            write_end.send_write_request_buffered(id, handle, offset, Cow::Borrowed(buf))
        })?
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
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file does not support writing",
            )));
        }

        let this = &mut *self;

        if this.write_futures.is_empty() {
            return Poll::Ready(Ok(()));
        }

        // flush only if there is pending awaitable writes
        if this.need_flush {
            this.inner.sftp().trigger_flushing();
            this.inner.need_flush = false;
        }

        this.write_cancellation_future
            .poll_for_task_failure(cx, this.inner.get_auxiliary())?;

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
            this.inner
                .inner
                .cache_id_mut(res.map_err(sftp_to_io_error)?.0);
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

        let max_write_len = self.max_write_len() as usize;

        let (n, bufs, buf) = if let Some(res) = take_io_slices(bufs, max_write_len) {
            res
        } else {
            return Poll::Ready(Ok(0));
        };

        let buffers = [bufs, &buf];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        let future = send_request(&mut this.inner, |write_end, id, handle, offset| {
            write_end.send_write_request_buffered_vectored2(id, handle, offset, &buffers)
        })?
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
