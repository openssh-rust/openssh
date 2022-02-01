use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::process::ExitStatus;
use std::time::Duration;

use openssh_sftp_client::{connect, Extensions, Limits, ReadEnd};
use thread_local::ThreadLocal;
use tokio::{task, time, try_join};

mod cache;
use cache::Cache;

mod file;
pub use file::{File, OpenOptions};

type Buffer = Vec<u8>;

type WriteEnd = openssh_sftp_client::WriteEnd<Buffer>;
type SharedData = openssh_sftp_client::SharedData<Buffer>;
type Id = openssh_sftp_client::Id<Buffer>;

/// Duration to wait before flushing the write buffer.
const FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    session: &'s Session,
    child: RemoteChildImp,

    shared_data: SharedData,
    flush_task: task::JoinHandle<Result<(), Error>>,
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

        let shared_data = SharedData::clone(&write_end);
        let flush_task = task::spawn(async move {
            let mut interval = time::interval(FLUSH_INTERVAL);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

            // The loop can only return `Err`
            loop {
                interval.tick().await;
                shared_data.flush().await?;
            }
        });

        let shared_data = SharedData::clone(&write_end);
        let read_task = task::spawn(async move {
            let mut read_end = read_end;

            loop {
                let new_requests_submit = read_end.wait_for_new_request().await;
                if new_requests_submit == 0 {
                    // All responses is read in and there is no
                    // write_end/shared_data left.
                    break Ok::<_, Error>(());
                }

                try_join!(
                    async {
                        // There is only 5 references to the shared data:
                        //  - the read end
                        //  - the shared data stored in read_task
                        //  - the shared data stored in flush_task
                        //  - the shared data stored in sftp
                        //  - one write_end
                        //
                        // In this case, the buffer should be flushed since
                        // it will not be able to group any writes.
                        if shared_data.strong_count() <= 5 {
                            shared_data.flush().await?;
                        }

                        Ok::<_, Error>(())
                    },
                    async {
                        // If attempt to read in more than new_requests_submit, then
                        // `read_in_one_packet` might block forever.
                        for _ in 0..new_requests_submit {
                            read_end.read_in_one_packet().await?;
                        }

                        Ok::<_, Error>(())
                    }
                )?;
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

            shared_data: write_end.into_shared_data(),
            read_task,
            flush_task,

            extensions,
            limits,

            thread_local_cache,
        })
    }

    /// Close sftp connection
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(self) -> Result<(), Error> {
        // Try to flush the data
        self.shared_data.flush().await?;
        // Wait for responses for all requests buffered and sent.
        self.read_task.await??;

        // terminate flush task only after all data is flushed.
        self.flush_task.abort();
        match self.flush_task.await {
            Ok(res) => res?,
            Err(join_err) => {
                if !join_err.is_cancelled() {
                    return Err(join_err.into());
                }
            }
        }

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
        WriteEnd::new(self.shared_data.clone())
    }

    pub(crate) fn get_thread_local_cached_id(&self) -> Id {
        self.thread_local_cache
            .get()
            .and_then(Cache::take)
            .unwrap_or_else(|| self.shared_data.create_response_id())
    }

    /// Give back id to the thread local cache.
    pub(crate) fn cache_id(&self, id: Id) {
        self.thread_local_cache.get_or(|| Cache::new(None)).set(id);
    }

    pub fn open_options(&self) -> OpenOptions<'_, '_> {
        OpenOptions::new(self)
    }

    /// Forcibly flush the write buffer.
    ///
    /// By default, it is flushed every 10ms.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn flush(&self) -> Result<(), Error> {
        self.shared_data.flush().await?;

        Ok(())
    }
}
