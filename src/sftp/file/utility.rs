use super::super::Auxiliary;

use std::fmt;
use std::future::Future;
use std::io::{self, IoSlice};
use std::mem;
use std::ops::Deref;
use std::pin::Pin;
use std::slice;
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio_util::sync::WaitForCancellationFuture;

use openssh_sftp_client::Error as SftpError;

const WAIT_FOR_CANCELLATION_FUTURE_SIZE: usize =
    mem::size_of::<WaitForCancellationFuture<'static>>();

#[repr(transparent)]
#[derive(Default)]
pub(super) struct SelfRefWaitForCancellationFuture(
    /// WaitForCancellationFuture is erased to an array
    /// since it is a holds a reference to `Auxiliary::cancel_token`,
    /// which lives as long as `Self`.
    ///
    /// WaitForCancellationFuture is boxed since it stores an intrusive node
    /// inline, which is removed from waitlist on drop.
    ///
    /// However, in rust, leaking is permitted, thus we have to box it.
    Option<Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>>>,
);

impl fmt::Debug for SelfRefWaitForCancellationFuture {
    fn fmt<'this>(&'this self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let future = self.0.as_ref().map(
            |reference: &Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>>| {
                let reference: &[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE] = &*reference;

                // safety:
                //  - The box is used to store WaitForCancellationFuture<'this>
                //  - &[u8; _] and &WaitForCancellationFuture has the same size
                let future: &WaitForCancellationFuture<'this> =
                    unsafe { mem::transmute(reference) };

                future
            },
        );

        f.debug_tuple("SelfRefWaitForCancellationFuture")
            .field(&future)
            .finish()
    }
}

impl Drop for SelfRefWaitForCancellationFuture {
    fn drop(&mut self) {
        debug_assert!(self.0.is_none());
    }
}

impl SelfRefWaitForCancellationFuture {
    /// This function must be called once and exactly once in `Drop` implementation.
    pub(super) unsafe fn drop<'this>(&'this mut self) {
        if let Some(boxed) = self.0.take() {
            // transmute the box to avoid moving `WaitForCancellationFuture`
            //
            // safety:
            //  - The box is used to store WaitForCancellationFuture<'this>
            //  - [u8; _] and WaitForCancellationFuture has the same size
            let _: Box<WaitForCancellationFuture<'this>> = mem::transmute(boxed);
        }
    }

    fn error() -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            SftpError::BackgroundTaskFailure(&"read/flush task failed"),
        )
    }

    /// Return `Ok(())` if the task hasn't failed yet and the context has
    /// already been registered.
    pub(super) fn poll_for_task_failure<'this, 'auxiliary: 'this>(
        &'this mut self,
        cx: &mut Context<'_>,
        auxiliary: &'auxiliary Auxiliary,
    ) -> Result<(), io::Error> {
        if self.0.is_none() {
            let cancel_token = &auxiliary.cancel_token;

            if cancel_token.is_cancelled() {
                return Err(Self::error());
            }

            let future: WaitForCancellationFuture<'this> = cancel_token.cancelled();
            // safety:
            //  - The box is used to store WaitForCancellationFuture<'this>
            //  - [u8; _] and WaitForCancellationFuture has the same size
            self.0 = Some(Box::pin(unsafe { mem::transmute(future) }));
        }

        {
            let reference: &mut Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>> =
                self.0.as_mut().expect("self.0 is just set to Some");

            let reference: Pin<&mut [u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]> = Pin::new(reference);

            // safety:
            //  - The box is used to store WaitForCancellationFuture<'this>
            //  - &mut [u8; _] and &mut WaitForCancellationFuture has the same size
            let future: Pin<&mut WaitForCancellationFuture<'this>> =
                unsafe { mem::transmute(reference) };

            match future.poll(cx) {
                Poll::Ready(_) => (),
                Poll::Pending => return Ok(()),
            }
        }

        self.0 = None;

        Err(Self::error())
    }
}

/// Return `Some((n, subslices, reminder))` where
///  - `n` is number of bytes in `subslices` and `reminder`.
///  - `subslices` is a subslice of `bufs`
///  - `reminder` might be a slice of `bufs[subslices.len()]`
///    if `subslices.len() < bufs.len()` and the total number
///    of bytes in `subslices` is less than `limit`.
///
/// Return `None` if the total number of bytes in `bufs` is empty.
fn take_slices<T: Deref<Target = [u8]>>(
    bufs: &'_ [T],
    limit: usize,
    create_slice: impl FnOnce(&T, usize) -> T,
) -> Option<(usize, &'_ [T], [T; 1])> {
    if bufs.is_empty() {
        return None;
    }

    let mut end = 0;
    let mut n = 0;

    // loop 'buf
    //
    // This loop would skip empty `IoSlice`s.
    for buf in bufs {
        let cnt = n + buf.len();

        // branch '1
        if cnt > limit {
            break;
        }

        n = cnt;
        end += 1;
    }

    let buf = if end < bufs.len() {
        n = limit;

        // In this branch, the loop 'buf terminate due to branch '1,
        // thus
        //
        //     n + buf.len() > limit,
        //     buf.len() > limit - n.
        //
        // And (limit - n) also cannot be 0, otherwise
        // branch '1 will not be executed.
        [create_slice(&bufs[end], limit - n)]
    } else {
        if n == 0 {
            return None;
        }

        [create_slice(&bufs[0], 0)]
    };

    Some((n, &bufs[..end], buf))
}

fn create_io_subslice<'a>(io_slice: &IoSlice<'a>, end: usize) -> IoSlice<'a> {
    let slice = &io_slice[..end];

    let ptr = slice.as_ptr();
    let len = slice.len();

    // safety: `IoSlice<'a>` holds a `&'a [u8]` internally.
    //
    // `IoSlice::deref` returns the internal slice, however due to limitations
    // of `Deref` trait, the lifetime is further reduced to lifetime of
    // `IoSlice`.
    //
    // The unsafe used here merely reverted it back to its original lifetime.
    let slice: &'a [u8] = unsafe { slice::from_raw_parts(ptr, len) };

    IoSlice::new(slice)
}

/// Return `Some((n, io_subslices, [reminder]))` where
///  - `n` is number of bytes in `io_subslices` and `reminder`.
///  - `io_subslices` is a subslice of `io_slices`
///  - `reminder` might be a slice of `io_slices[io_subslices.len()]`
///    if `io_subslices.len() < io_slices.len()` and the total number
///    of bytes in `io_subslices` is less than `limit`.
///
/// Return `None` if the total number of bytes in `io_slices` is empty.
pub(super) fn take_io_slices<'a>(
    io_slices: &'a [IoSlice<'a>],
    limit: usize,
) -> Option<(usize, &'a [IoSlice<'a>], [IoSlice<'a>; 1])> {
    take_slices(io_slices, limit, create_io_subslice)
}

/// Return `Some((n, bytes_subslice, [reminder]))` where
///  - `n` is number of bytes in `bytes_subslice` and `reminder`.
///  - `bytes_subslice` is a subslice of `bytes_slice`
///  - `reminder` might be a slice of `bytes_slice[bytes_subslice.len()]`
///    if `bytes_subslice.len() < bytes_slice.len()` and the total number
///    of bytes in `bytes_subslice` is less than `limit`.
///
/// Return `None` if the total number of bytes in `bytes_slice` is empty.
pub(super) fn take_bytes(
    bytes_slice: &[Bytes],
    limit: usize,
) -> Option<(usize, &[Bytes], [Bytes; 1])> {
    take_slices(bytes_slice, limit, |bytes, end| bytes.slice(0..end))
}
