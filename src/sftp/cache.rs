use super::{Auxiliary, BoxedWaitForCancellationFuture, Error, Id, Sftp, SharedData, WriteEnd};

use std::any::type_name;
use std::cell::Cell;
use std::fmt;
use std::future::Future;
use std::ops::{Deref, DerefMut};

use openssh_sftp_client::Error as SftpError;

#[repr(transparent)]
pub(super) struct Cache<T>(Cell<Option<T>>);

impl<T> Cache<T> {
    pub(super) const fn new(value: Option<T>) -> Self {
        Self(Cell::new(value))
    }

    pub(super) fn take(&self) -> Option<T> {
        self.0.take()
    }

    pub(super) fn set(&self, value: T) {
        self.0.set(Some(value));
    }
}

impl<T> fmt::Debug for Cache<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cache<{}>", type_name::<T>())
    }
}

trait IdCacher {
    fn get_thread_local_cached_id(&self) -> Id;

    /// Give back id to the thread local cache.
    fn cache_id(&self, id: Id);
}

impl IdCacher for SharedData {
    fn get_thread_local_cached_id(&self) -> Id {
        self.get_auxiliary()
            .thread_local_cache
            .get()
            .and_then(Cache::take)
            .unwrap_or_else(|| self.create_response_id())
    }

    /// Give back id to the thread local cache.
    fn cache_id(&self, id: Id) {
        self.get_auxiliary()
            .thread_local_cache
            .get_or(|| Cache::new(None))
            .set(id);
    }
}

#[derive(Debug)]
pub(super) struct WriteEndWithCachedId<'s> {
    sftp: &'s Sftp<'s>,
    inner: WriteEnd,
    id: Option<Id>,
    /// WaitForCancellationFuture adds itself as an entry to the internal
    /// linked list of CancellationToken when `poll`ed.
    ///
    /// Thus, in its `Drop::drop` implementation, it is removed from the
    /// linked list.
    ///
    /// However, rust does not guarantee on 'no leaking', thus it is possible
    /// and safe for user to `mem::forget` the future returned, and thus
    /// causing the linked list to point to invalid memory locations.
    ///
    /// To avoid this, we have to box this future.
    ///
    /// However, allocate a new box each time a future is called is super
    /// expensive, thus we keep it cached so that we can reuse it.
    wait_for_cancell_future: BoxedWaitForCancellationFuture<'s>,
}

impl Clone for WriteEndWithCachedId<'_> {
    fn clone(&self) -> Self {
        Self {
            sftp: self.sftp,
            inner: self.inner.clone(),
            id: None,
            wait_for_cancell_future: BoxedWaitForCancellationFuture::new(),
        }
    }
}

impl Deref for WriteEndWithCachedId<'_> {
    type Target = WriteEnd;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WriteEndWithCachedId<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for WriteEndWithCachedId<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.cache_id(id);
        }
    }
}

impl<'s> WriteEndWithCachedId<'s> {
    pub(super) fn new(sftp: &'s Sftp<'s>, inner: WriteEnd) -> Self {
        Self {
            sftp,
            inner,
            id: None,
            wait_for_cancell_future: BoxedWaitForCancellationFuture::new(),
        }
    }

    pub(super) fn get_id_mut(&mut self) -> Id {
        self.id
            .take()
            .unwrap_or_else(|| self.inner.get_thread_local_cached_id())
    }

    pub(super) fn cache_id(&self, id: Id) {
        self.inner.cache_id(id);
    }

    pub(super) fn cache_id_mut(&mut self, id: Id) {
        if self.id.is_none() {
            self.id = Some(id);
        } else {
            self.cache_id(id);
        }
    }

    /// * `f` - the future must be cancel safe.
    pub(super) async fn cancel_if_task_failed<R, E, F>(&mut self, future: F) -> Result<R, Error>
    where
        F: Future<Output = Result<R, E>>,
        E: Into<Error>,
    {
        let cancel_err = || Err(BoxedWaitForCancellationFuture::cancel_error().into());
        let auxiliary = self.sftp.shared_data.get_auxiliary();

        if auxiliary.cancel_token.is_cancelled() {
            return cancel_err();
        }

        tokio::select! {
            res = future => res.map_err(Into::into),
            _ = self.wait_for_cancell_future.wait(auxiliary) => cancel_err(),
        }
    }

    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        let id = self.get_id_mut();
        let write_end = &mut self.inner;

        let future = f(write_end, id)?;

        // Requests is already added to write buffer, so wakeup
        // the `flush_task`.
        self.get_auxiliary().wakeup_flush_task();

        let (id, ret) = self.cancel_if_task_failed(future).await?;

        self.cache_id_mut(id);

        Ok(ret)
    }

    pub(super) fn get_auxiliary(&self) -> &'s Auxiliary {
        self.sftp.shared_data.get_auxiliary()
    }

    pub(super) fn sftp(&self) -> &'s Sftp<'s> {
        self.sftp
    }
}
