use super::super::{Buffer, Data};
use super::{utility::SelfRefWaitForCancellationFuture, File, SftpError};

use std::borrow::Cow;
use std::cmp::min;
use std::collections::VecDeque;
use std::future::Future;
use std::io::{self, IoSlice};
use std::mem;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

use openssh_sftp_client::{AwaitableDataFuture, AwaitableStatusFuture};

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

/// File that implements [`AsyncRead`], [`AsyncSeek`] and [`AsyncWrite`],
/// that is compatible with [`tokio::fs::File`].
#[derive(Debug)]
pub struct TokioCompactFile<'s> {
    inner: File<'s>,

    buffer: Vec<u8>,

    read_future: Option<AwaitableDataFuture<Buffer>>,
    read_cancellation_future: SelfRefWaitForCancellationFuture,

    write_futures: VecDeque<AwaitableStatusFuture<Buffer>>,
    write_cancellation_future: SelfRefWaitForCancellationFuture,
}

impl<'s> TokioCompactFile<'s> {
    /// Create a [`TokioCompactFile`].
    pub fn new(inner: File<'s>) -> Self {
        Self {
            inner,

            buffer: Vec::new(),

            read_future: None,
            read_cancellation_future: SelfRefWaitForCancellationFuture::default(),

            write_futures: VecDeque::new(),
            write_cancellation_future: SelfRefWaitForCancellationFuture::default(),
        }
    }
}

impl<'s> From<File<'s>> for TokioCompactFile<'s> {
    fn from(inner: File<'s>) -> Self {
        Self::new(inner)
    }
}

impl Drop for TokioCompactFile<'_> {
    fn drop(&mut self) {
        unsafe {
            self.read_cancellation_future.drop();
            self.write_cancellation_future.drop();
        }
    }
}

/// Creates a new [`TokioCompactFile`] instance that shares the
/// same underlying file handle as the existing File instance.
///
/// Reads, writes, and seeks can be performed independently.
impl Clone for TokioCompactFile<'_> {
    fn clone(&self) -> Self {
        Self::new(self.inner.clone())
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
        use io::SeekFrom::*;

        match position {
            Start(pos) => {
                if pos == self.offset {
                    return Ok(());
                }
            }
            Current(n) => {
                if n == 0 {
                    return Ok(());
                }
            }
            _ => (),
        }

        Pin::new(&mut self.inner).start_seek(position)?;

        // Reset future since they are invalidated by change of offset.
        self.read_future = None;

        Ok(())
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Pin::new(&mut self.inner).poll_complete(cx)
    }
}

/// [`TokioCompactFile`] can read in at most [`File::max_read_len`] bytes.
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
            // Get id, buffer and offset to avoid reference to this.
            let id = this.get_id_mut();
            let buffer = mem::take(&mut this.buffer);
            let offset = this.offset;

            // Reference it here to make it clear that we are
            // using different part of Self.
            let write_end = &mut this.inner.write_end;
            let handle = &this.inner.handle;

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

            // Store it in this.read_future
            this.read_future = Some(future);
            this.read_future
                .as_mut()
                .expect("FileFuture::Data is just assigned to self.future!")
        };

        let auxiliary = this.inner.write_end.get_auxiliary();

        this.read_cancellation_future
            .poll_for_task_failure(cx, auxiliary)?;

        // Wait for the future
        let (id, data) = ready!(Pin::new(future).poll(cx)).map_err(sftp_to_io_error)?;

        this.cache_id_mut(id);
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

        // Reuse the buffer
        if buffer.capacity() >= this.buffer.capacity() {
            this.buffer = buffer;
        }

        // Adjust offset and reset this.future
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
///
/// [`TokioCompactFile`] can read in at most [`File::max_write_len`] bytes.
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

        // Get id, buffer and offset to avoid reference to this.
        let id = this.get_id_mut();
        let offset = this.offset;

        // Reference it here to make it clear that we are
        // using different part of Self.
        let write_end = &mut this.inner.write_end;
        let handle = &this.inner.handle;

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
            // WriteEnd::flush return true if flush succeeds, false if not.
            //
            // If it succeeds, then we no longer need to flush it.
            this.inner.need_flush = !ready!(
                // Future returned by WriteEnd::flush does not contain
                // self-reference, so it can be optimized and placed
                // on stack.
                //
                // It is also cancel safe, so we don't need to store it.
                Pin::new(&mut Box::pin(this.inner.write_end.flush())).poll(cx)
            )?;
        }

        let auxiliary = this.inner.write_end.get_auxiliary();

        this.write_cancellation_future
            .poll_for_task_failure(cx, auxiliary)?;

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

        let max_write_len = self.max_write_len() as usize;

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
        let write_end = &mut this.inner.write_end;
        let handle = &this.inner.handle;

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
