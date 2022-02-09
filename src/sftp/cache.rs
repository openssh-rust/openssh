use super::{Error, Id, SharedData, WriteEnd};

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
pub(super) struct WriteEndWithCachedId(WriteEnd, Option<Id>);

impl From<WriteEnd> for WriteEndWithCachedId {
    fn from(write_end: WriteEnd) -> Self {
        Self(write_end, None)
    }
}

impl Clone for WriteEndWithCachedId {
    fn clone(&self) -> Self {
        self.0.clone().into()
    }
}

impl Deref for WriteEndWithCachedId {
    type Target = WriteEnd;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WriteEndWithCachedId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for WriteEndWithCachedId {
    fn drop(&mut self) {
        if let Some(id) = self.1.take() {
            self.cache_id(id);
        }
    }
}

impl WriteEndWithCachedId {
    pub(super) fn get_id_mut(&mut self) -> Id {
        self.1
            .take()
            .unwrap_or_else(|| self.0.get_thread_local_cached_id())
    }

    pub(super) fn cache_id(&self, id: Id) {
        self.0.cache_id(id);
    }

    pub(super) fn cache_id_mut(&mut self, id: Id) {
        if self.1.is_none() {
            self.1 = Some(id);
        } else {
            self.cache_id(id);
        }
    }

    /// * `f` - the future must be cancel safe.
    pub(super) async fn cancel_if_task_failed<R, E, F>(&self, future: F) -> Result<R, Error>
    where
        F: Future<Output = Result<R, E>>,
        E: Into<Error>,
    {
        tokio::select! {
            res = future => res.map_err(Into::into),
            _ = self.0.get_auxiliary().cancel_token.cancelled() => Err(
                SftpError::BackgroundTaskFailure(&"read/flush task failed").into()
            ),
        }
    }

    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        let id = self.get_id_mut();
        let write_end = &mut self.0;

        let future = f(write_end, id)?;

        // Requests is already added to write buffer, so wakeup
        // the `flush_task`.
        write_end.get_auxiliary().wakeup_flush_task();

        let (id, ret) = self.cancel_if_task_failed(future).await?;

        self.cache_id_mut(id);

        Ok(ret)
    }
}
