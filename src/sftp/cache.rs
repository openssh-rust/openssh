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
