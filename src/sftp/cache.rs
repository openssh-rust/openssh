use super::{Id, SharedData};

use std::any::type_name;
use std::cell::Cell;
use std::fmt;

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

pub(crate) trait IdCacher {
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
