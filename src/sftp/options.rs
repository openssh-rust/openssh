use std::time::Duration;

/// Options when creating [`super::Sftp`].
#[derive(Debug, Copy, Clone, Default)]
pub struct SftpOptions {
    flush_interval: Option<Duration>,
    max_read_len: Option<u32>,
    max_write_len: Option<u32>,
}

impl SftpOptions {
    /// Create a new [`SftpOptions`].
    pub const fn new() -> Self {
        Self {
            flush_interval: None,
            max_read_len: None,
            max_write_len: None,
        }
    }

    /// Set `flush_interval`, default value is 0.5 ms.
    ///
    /// `flush_interval` decides the maximum time your requests would stay
    /// in the write buffer before it is actually sent to the remote.
    ///
    /// If another thread is doing flushing, then the internal `flush_task`
    /// [`super::Sftp`] started would wait for another `flush_interval`.
    ///
    /// Setting it to be larger might improve overall performance by grouping
    /// writes and reducing the overhead of packet sent over network, but it
    /// might also increase latency, so be careful when setting the
    /// `flush_interval`.
    ///
    /// If `flush_interval` is set to 0, then every packet
    /// is flushed immediately.
    ///
    /// NOTE that it is perfectly OK to set `flush_interval` to 0 and
    /// it would not slowdown the program, as flushing is only performed
    /// on daemon.
    pub const fn flush_interval(mut self, flush_interval: Duration) -> Self {
        self.flush_interval = Some(flush_interval);
        self
    }

    pub(super) fn get_flush_interval(&self) -> Duration {
        self.flush_interval
            .unwrap_or_else(|| Duration::from_micros(500))
    }

    /// Set `max_read_len`.
    ///
    /// It can be used to reduce `max_read_len`, but cannot be used
    /// to increase `max_read_len`.
    pub const fn max_read_len(mut self, max_read_len: u32) -> Self {
        self.max_read_len = Some(max_read_len);
        self
    }

    pub(super) fn get_max_read_len(&self) -> Option<u32> {
        self.max_read_len
    }

    /// Set `max_write_len`.
    ///
    /// It can be used to reduce `max_write_len`, but cannot be used
    /// to increase `max_write_len`.
    pub const fn max_write_len(mut self, max_write_len: u32) -> Self {
        self.max_write_len = Some(max_write_len);
        self
    }

    pub(super) fn get_max_write_len(&self) -> Option<u32> {
        self.max_write_len
    }
}
