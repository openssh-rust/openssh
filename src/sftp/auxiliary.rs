use super::{Cache, Id};

use once_cell::sync::OnceCell;
use openssh_sftp_client::Extensions;
use std::sync::atomic::{AtomicUsize, Ordering};
use thread_local::ThreadLocal;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Copy, Clone)]
pub(super) struct Limits {
    pub(super) read_len: u32,
    pub(super) write_len: u32,
}

#[derive(Debug)]
pub(super) struct ConnInfo {
    pub(super) limits: Limits,
    pub(super) extensions: Extensions,
    pub(super) max_pending_requests: usize,
}

#[derive(Debug)]
pub(super) struct Auxiliary {
    pub(super) conn_info: OnceCell<ConnInfo>,

    pub(super) thread_local_cache: ThreadLocal<Cache<Id>>,

    /// cancel_token is used to cancel `Awaitable*Future`
    /// when the read_task/flush_task has failed.
    pub(super) cancel_token: CancellationToken,

    /// flush_end_notify is used to avoid unnecessary wakeup
    /// in flush_task.
    pub(super) flush_end_notify: Notify,

    pub(super) pending_requests: AtomicUsize,

    /// `Notify::notify_one` is called if
    /// pending_requests == max_pending_requests.
    pub(super) requests_too_many_notify: Notify,
}

impl Auxiliary {
    pub(super) fn new() -> Self {
        Self {
            conn_info: OnceCell::new(),
            thread_local_cache: ThreadLocal::new(),
            cancel_token: CancellationToken::new(),
            flush_end_notify: Notify::new(),

            pending_requests: AtomicUsize::new(0),
            requests_too_many_notify: Notify::new(),
        }
    }

    pub(super) fn wakeup_flush_task(&self) {
        self.flush_end_notify.notify_one();

        let max_pending_requests = self.conn_info().max_pending_requests;

        // Use `==` here to avoid unnecessary wakeup of flush_task.
        if self.pending_requests.fetch_add(1, Ordering::Relaxed) == max_pending_requests {
            self.requests_too_many_notify.notify_one();
        }
    }

    pub(super) fn consume_pending_requests(&self, requests_consumed: usize) {
        let max_pending_requests = self.conn_info().max_pending_requests;

        // If pending_requests is still greater than max_pending_request
        // (might be caused by new requests), then wakeup flush_task.
        if self
            .pending_requests
            .fetch_sub(requests_consumed, Ordering::Relaxed)
            >= max_pending_requests
        {
            self.requests_too_many_notify.notify_one();
        }
    }

    fn conn_info(&self) -> &ConnInfo {
        self.conn_info
            .get()
            .expect("auxiliary.conn_info shall be initialized by sftp::Sftp::new")
    }

    pub(super) fn extensions(&self) -> Extensions {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info().extensions
    }

    pub(super) fn limits(&self) -> Limits {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info().limits
    }
}
