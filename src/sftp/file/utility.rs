use super::super::{Auxiliary, Error};

use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_util::sync::WaitForCancellationFuture;

use openssh_sftp_client::Error as SftpError;

const WAIT_FOR_CANCELLATION_FUTURE_SIZE: usize =
    mem::size_of::<WaitForCancellationFuture<'static>>();

#[derive(Debug, Default)]
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

impl SelfRefWaitForCancellationFuture {
    /// This function must be called once in `Drop` implementation.
    pub(super) unsafe fn drop<'this>(&'this mut self) {
        if let Some(boxed) = self.0.take() {
            let _: WaitForCancellationFuture<'this> = mem::transmute(*boxed);
        }
    }

    fn error() -> Error {
        SftpError::BackgroundTaskFailure(&"read/flush task failed").into()
    }

    /// Return `Ok(())` if the task hasn't failed yet and the context has
    /// already been registered.
    pub(super) fn poll_for_task_failure<'this, 'auxiliary: 'this>(
        &'this mut self,
        cx: &mut Context<'_>,
        auxiliary: &'auxiliary Auxiliary,
    ) -> Result<(), Error> {
        if self.0.is_none() {
            let cancel_token = &auxiliary.cancel_token;

            if cancel_token.is_cancelled() {
                return Err(Self::error());
            }

            let future: WaitForCancellationFuture<'this> = cancel_token.cancelled();
            self.0 = Some(Box::pin(unsafe { mem::transmute(future) }));
        }

        {
            let reference: &mut Pin<Box<[u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]>> =
                self.0.as_mut().expect("self.0 is just set to Some");

            let reference: Pin<&mut [u8; WAIT_FOR_CANCELLATION_FUTURE_SIZE]> = Pin::new(reference);

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
