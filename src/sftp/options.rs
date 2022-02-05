use std::time::Duration;

/// Options when creating [`super::Sftp`].
#[derive(Debug, Copy, Clone, Default)]
pub struct SftpOptions {
    flush_interval: Option<Duration>,
}

impl SftpOptions {
    /// Create a new [`SftpOptions`].
    pub const fn new() -> Self {
        Self {
            flush_interval: None,
        }
    }

    /// Set flush_interval.
    pub const fn flush_interval(mut self, flush_interval: Duration) -> Self {
        self.flush_interval = Some(flush_interval);
        self
    }

    pub(super) fn get_flush_interval(&self) -> Duration {
        self.flush_interval
            .unwrap_or_else(|| Duration::from_micros(500))
    }
}
