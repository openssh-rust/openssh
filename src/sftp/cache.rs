use std::cell::Cell;

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
