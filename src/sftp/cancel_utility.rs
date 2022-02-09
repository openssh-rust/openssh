use super::{Auxiliary, Sftp, SftpError};

use std::fmt;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_util::sync::WaitForCancellationFuture;

const WAIT_FOR_CANCELLATION_FUTURE_SIZE: usize =
    mem::size_of::<WaitForCancellationFuture<'static>>();

/// lifetime 's is reference to `sftp::Sftp`
///
/// # Safety
///
/// As long as `sftp::Sftp` is valid, the cancellation token it references
/// to must be kept valid by `sftp::Sftp::SharedData`.
#[repr(transparent)]
pub(super) struct SelfRefWaitForCancellationFuture<'s>(
    /// WaitForCancellationFuture is erased to an array
    /// since it is a holds a reference to `Auxiliary::cancel_token`,
    /// which lives as long as `Self`.
    ///
    /// WaitForCancellationFuture is boxed since it stores an intrusive node
    /// inline, which is removed from waitlist on drop.
    ///
    /// However, in rust, leaking is permitted, thus we have to box it.
    Option<Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>>>,
    PhantomData<&'s Sftp<'s>>,
);

impl fmt::Debug for SelfRefWaitForCancellationFuture<'_> {
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

impl Drop for SelfRefWaitForCancellationFuture<'_> {
    fn drop<'this>(&'this mut self) {
        if let Some(pinned_boxed) = self.0.take() {
            let ptr = Box::into_raw(
                Pin::<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>>::into_inner(pinned_boxed),
            );

            // transmute the box to avoid moving `WaitForCancellationFuture`
            //
            // safety:
            //  - The box is used to store WaitForCancellationFuture<'this>
            //  - [u8; _] and WaitForCancellationFuture has the same size
            let _: Box<WaitForCancellationFuture<'this>> =
                unsafe { Box::from_raw(ptr as *mut WaitForCancellationFuture<'this>) };
        }
    }
}

impl SelfRefWaitForCancellationFuture<'_> {
    /// # Safety
    ///
    /// lifetime `'s` must be the same as `&'s Sftp<'s>`.
    pub(super) unsafe fn new() -> Self {
        Self(None, PhantomData)
    }

    fn error() -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            SftpError::BackgroundTaskFailure(&"read/flush task failed"),
        )
    }

    fn init_future_if_needed<'this, 'auxiliary: 'this>(
        &'this mut self,
        auxiliary: &'auxiliary Auxiliary,
    ) {
        if self.0.is_none() {
            let cancel_token = &auxiliary.cancel_token;

            let future: WaitForCancellationFuture<'this> = cancel_token.cancelled();
            // safety:
            //  - The box is used to store WaitForCancellationFuture<'this>
            //  - [u8; _] and WaitForCancellationFuture has the same size
            self.0 = Some(Box::pin(unsafe { mem::transmute(future) }));
        }
    }

    pub(super) fn get_wait_for_cancel_future<'this, 'auxiliary: 'this>(
        &'this mut self,
        auxiliary: &'auxiliary Auxiliary,
    ) -> Pin<&mut WaitForCancellationFuture<'this>> {
        self.init_future_if_needed(auxiliary);

        let reference: &mut Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>> =
            self.0.as_mut().expect("self.0 is just set to Some");

        let reference: Pin<&mut [u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]> = Pin::new(reference);

        // safety:
        //  - The box is used to store WaitForCancellationFuture<'this>
        //  - &mut [u8; _] and &mut WaitForCancellationFuture has the same size
        let future: Pin<&mut WaitForCancellationFuture<'this>> =
            unsafe { mem::transmute(reference) };

        future
    }

    /// Return `Ok(())` if the task hasn't failed yet and the context has
    /// already been registered.
    pub(super) fn poll_for_task_failure<'this, 'auxiliary: 'this>(
        &'this mut self,
        cx: &mut Context<'_>,
        auxiliary: &'auxiliary Auxiliary,
    ) -> Result<(), io::Error> {
        if auxiliary.cancel_token.is_cancelled() {
            return Err(Self::error());
        }

        match self.get_wait_for_cancel_future(auxiliary).poll(cx) {
            Poll::Ready(_) => {
                self.0 = None;

                Err(Self::error())
            }
            Poll::Pending => Ok(()),
        }
    }
}
