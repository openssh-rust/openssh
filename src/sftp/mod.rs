use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::process::ExitStatus;
use std::time::Duration;

use openssh_sftp_client::{connect, Extensions, Limits, ReadEnd};
use thread_local::ThreadLocal;
use tokio::{task, time};

mod cache;
use cache::Cache;

mod file;
pub use file::{File, OpenOptions};

type WriteEnd = openssh_sftp_client::WriteEnd<Vec<u8>>;
type Id = openssh_sftp_client::Id<Vec<u8>>;

/// Duration to wait before flushing the write buffer.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(10);

async fn flush_if_necessary(
    flushed: &mut bool,
    read_end: &mut ReadEnd<Vec<u8>>,
) -> Result<(), Error> {
    if !*flushed {
        // New requests are now in the write buffer and might be
        // flushed.
        // Wait for new response and flush the buffer if timeout.
        match time::timeout(FLUSH_TIMEOUT, read_end.ready_for_read()).await {
            Ok(res) => res?,
            Err(_) => *flushed = read_end.flush_write_end_buffer().await?,
        };
    }

    Ok(())
}

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    session: &'s Session,
    child: RemoteChildImp,

    write_end: WriteEnd,
    read_task: task::JoinHandle<Result<(), Error>>,

    extensions: Extensions,
    limits: Limits,

    thread_local_cache: ThreadLocal<Cache<Id>>,
}

impl<'s> Sftp<'s> {
    pub(crate) async fn new(
        session: &'s Session,
        child: RemoteChildImp,
        stdin: ChildStdin,
        stdout: ChildStdout,
    ) -> Result<Sftp<'s>, Error> {
        let (mut write_end, read_end, extensions) = connect(stdout, stdin).await?;
        let read_task = task::spawn(async move {
            let mut read_end = read_end;

            loop {
                let new_requests_submit = read_end.wait_for_new_request().await;
                if new_requests_submit == 0 {
                    break Ok::<_, Error>(());
                }

                // whether all of the `new_requests_submit` pending requests is
                // flushed.
                let mut flushed = false;

                // If attempt to read in more than new_requests_submit, then
                // `read_in_one_packet` might block forever.
                for _ in 0..new_requests_submit {
                    flush_if_necessary(&mut flushed, &mut read_end).await?;

                    read_end.read_in_one_packet().await?;
                }
            }
        });

        let id = write_end.create_response_id();

        let (id, limits) = if extensions.limits {
            let awaitable = write_end.send_limits_request(id)?;
            write_end.flush().await?;
            awaitable.wait().await?
        } else {
            (
                id,
                Limits {
                    packet_len: 0,
                    read_len: openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64,
                    write_len: openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64,
                    open_handles: 0,
                },
            )
        };

        let thread_local_cache = ThreadLocal::new();
        thread_local_cache.get_or(|| Cache::new(Some(id)));

        Ok(Self {
            session,
            child,

            write_end,
            read_task,

            extensions,
            limits,

            thread_local_cache,
        })
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        self.write_end.flush().await?;

        self.read_task.await??;

        let res: Result<ExitStatus, Error> =
            crate::child::delegate!(self.child, child, { child.wait().await });
        let exit_status = res?;

        if !exit_status.success() {
            Err(Error::SftpError(
                openssh_sftp_client::Error::SftpServerFailure(exit_status),
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn write_end(&self) -> WriteEnd {
        self.write_end.clone()
    }

    pub(crate) fn get_thread_local_cached_id(&self) -> Id {
        self.thread_local_cache
            .get()
            .and_then(Cache::take)
            .unwrap_or_else(|| self.write_end.create_response_id())
    }

    /// Give back id to the thread local cache.
    pub(crate) fn cache_id(&self, id: Id) {
        self.thread_local_cache.get_or(|| Cache::new(None)).set(id);
    }

    pub fn options(&self) -> OpenOptions<'_, '_> {
        OpenOptions::new(self)
    }
}
