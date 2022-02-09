use super::{Auxiliary, SftpError};

use std::fmt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_util::sync::WaitForCancellationFuture;

/// lifetime 's is reference to `sftp::Sftp`
///
/// # Safety
///
/// As long as `sftp::Sftp` is valid, the cancellation token it references
/// to must be kept valid by `sftp::Sftp::SharedData`.
#[repr(transparent)]
pub(super) struct BoxedWaitForCancellationFuture<'s>(
    /// WaitForCancellationFuture is erased to an array
    /// since it is a holds a reference to `Auxiliary::cancel_token`,
    /// which lives as long as `Self`.
    ///
    /// WaitForCancellationFuture is boxed since it stores an intrusive node
    /// inline, which is removed from waitlist on drop.
    ///
    /// However, in rust, leaking is permitted, thus we have to box it.
    Option<Pin<Box<WaitForCancellationFuture<'s>>>>,
);

impl fmt::Debug for BoxedWaitForCancellationFuture<'_> {
    fn fmt<'this>(&'this self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BoxedWaitForCancellationFuture")
            .field(&self.0.as_ref())
            .finish()
    }
}

impl<'s> BoxedWaitForCancellationFuture<'s> {
    /// # Safety
    ///
    /// lifetime `'s` must be the same as `&'s Sftp<'s>`.
    pub(super) fn new() -> Self {
        Self(None)
    }

    fn error() -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            SftpError::BackgroundTaskFailure(&"read/flush task failed"),
        )
    }

    pub(super) fn get_wait_for_cancel_future(
        &mut self,
        auxiliary: &'s Auxiliary,
    ) -> Pin<&mut WaitForCancellationFuture<'s>> {
        if self.0.is_none() {
            self.0 = Some(Box::pin(auxiliary.cancel_token.cancelled()));
        }

        self.0
            .as_mut()
            .expect("self.0 is just set to Some")
            .as_mut()
    }

    /// Return `Ok(())` if the task hasn't failed yet and the context has
    /// already been registered.
    pub(super) fn poll_for_task_failure(
        &mut self,
        cx: &mut Context<'_>,
        auxiliary: &'s Auxiliary,
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
