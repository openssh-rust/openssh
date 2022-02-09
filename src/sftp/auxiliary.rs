use super::{Cache, Id};

use openssh_sftp_client::Extensions;
use parking_lot::RwLock;
use thread_local::ThreadLocal;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default, Copy, Clone)]
pub(super) struct Limits {
    pub(super) read_len: u32,
    pub(super) write_len: u32,
}

#[derive(Debug, Default)]
pub(super) struct ConnInfo {
    pub(super) limits: Limits,
    pub(super) extensions: Extensions,
}

#[derive(Debug)]
pub(super) struct Auxiliary {
    pub(super) conn_info: RwLock<ConnInfo>,

    pub(super) thread_local_cache: ThreadLocal<Cache<Id>>,

    /// cancel_token is used to cancel `Awaitable*Future`
    /// when the read_task/flush_task has failed.
    pub(super) cancel_token: CancellationToken,

    /// flush_end_notify is used to avoid unnecessary wakeup
    /// in flush_task.
    pub(super) flush_end_notify: Notify,
}

impl Auxiliary {
    pub(super) fn new() -> Self {
        Self {
            conn_info: RwLock::new(ConnInfo::default()),
            thread_local_cache: ThreadLocal::new(),
            cancel_token: CancellationToken::new(),
            flush_end_notify: Notify::new(),
        }
    }

    pub(super) fn wakeup_flush_task(&self) {
        self.flush_end_notify.notify_one();
    }

    pub(super) fn extensions(&self) -> Extensions {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info.read().extensions
    }

    pub(super) fn limits(&self) -> Limits {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info.read().limits
    }
}
