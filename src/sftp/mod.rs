use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::cmp::min;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::Ordering;

use bytes::BytesMut;
use openssh_sftp_client::{connect_with_auxiliary, Error as SftpError};
use tokio::{task, time};
use tokio_util::sync::CancellationToken;

pub use openssh_sftp_client::{UnixTimeStamp, UnixTimeStampError};

mod cancel_utility;
use cancel_utility::BoxedWaitForCancellationFuture;

mod options;
pub use options::SftpOptions;

mod auxiliary;
use auxiliary::Auxiliary;

mod cache;
use cache::{Cache, WriteEndWithCachedId};

mod handle;
use handle::OwnedHandle;

mod file;
pub use file::TokioCompactFile;
pub use file::{File, OpenOptions};

mod fs;
pub use fs::DirEntry;
pub use fs::ReadDir;
pub use fs::{Dir, DirBuilder, Fs};

mod metadata;
pub use metadata::{FileType, MetaData, MetaDataBuilder, Permissions};

type Buffer = BytesMut;

type WriteEnd = openssh_sftp_client::WriteEnd<Buffer, Auxiliary>;
type SharedData = openssh_sftp_client::SharedData<Buffer, Auxiliary>;
type Id = openssh_sftp_client::Id<Buffer>;
type Data = openssh_sftp_client::Data<Buffer>;

async fn flush(shared_data: &SharedData) -> Result<(), Error> {
    Ok(shared_data.flush().await.map_err(SftpError::from)?)
}

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    phantom_data: PhantomData<&'s Session>,
    child: RemoteChildImp,

    shared_data: SharedData,
    flush_task: task::JoinHandle<Result<(), Error>>,
    read_task: task::JoinHandle<Result<(), Error>>,
}

impl<'s> Sftp<'s> {
    pub(crate) async fn new(
        child: RemoteChildImp,
        stdin: ChildStdin,
        stdout: ChildStdout,
        options: SftpOptions,
    ) -> Result<Sftp<'s>, Error> {
        let (mut write_end, mut read_end, extensions) =
            connect_with_auxiliary(stdout, stdin, Auxiliary::new()).await?;

        let id = write_end.create_response_id();

        let (id, read_len, write_len) = if extensions.limits {
            let awaitable = write_end.send_limits_request(id)?;

            flush(&write_end).await?;

            // Call wait_for_new_request to consume the pending new requests
            let new_requests_submit = read_end.wait_for_new_request().await;
            debug_assert_eq!(new_requests_submit, 1);

            read_end.read_in_one_packet().await?;

            let (id, mut limits) = awaitable.wait().await?;

            if limits.read_len == 0 {
                limits.read_len =
                    openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64;
            }

            if limits.write_len == 0 {
                limits.write_len =
                    openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64;
            }

            (id, limits.read_len, limits.write_len)
        } else {
            (
                id,
                openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64,
                openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64,
            )
        };

        // sftp can accept packet as large as u32::MAX,
        // however each read/write request also has a header and
        // it contains a handle, which is 4-byte long for openssh
        // but can be at most 256 bytes long for other implementations.

        let read_len = read_len.try_into().unwrap_or(u32::MAX - 300);
        let read_len = options
            .get_max_read_len()
            .map(|v| min(v, read_len))
            .unwrap_or(read_len);

        let write_len = write_len.try_into().unwrap_or(u32::MAX - 300);
        let write_len = options
            .get_max_write_len()
            .map(|v| min(v, write_len))
            .unwrap_or(write_len);

        let limits = auxiliary::Limits {
            read_len,
            write_len,
        };

        let auxiliary = write_end.get_auxiliary();

        auxiliary
            .conn_info
            .set(auxiliary::ConnInfo {
                limits,
                extensions,
                max_pending_requests: options.get_max_pending_requests(),
            })
            .expect("auxiliary.conn_info shall be empty");
        auxiliary.thread_local_cache.get_or(|| Cache::new(Some(id)));

        let shared_data = SharedData::clone(&write_end);
        let flush_interval = options.get_flush_interval();
        let flush_task = task::spawn(async move {
            let mut interval = time::interval(flush_interval);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

            let auxiliary = shared_data.get_auxiliary();
            let flush_end_notify = &auxiliary.flush_end_notify;
            let pending_requests = &auxiliary.pending_requests;
            let max_pending_requests = auxiliary.max_pending_requests();

            let _cancel_guard = auxiliary.cancel_token.clone().drop_guard();

            // The loop can only return `Err`
            loop {
                flush_end_notify.notified().await;

                tokio::select! {
                    _ = interval.tick() => (),
                    // tokio::sync::Notify is cancel safe, however
                    // cancelling it would lose the place in the queue.
                    //
                    // However, since flush_task is the only one who
                    // calls `flush_immediately.notified()`, it
                    // is totally fine to cancel here.
                    _ = auxiliary.flush_immediately.notified() => (),
                };

                let mut prev_pending_requests = pending_requests.load(Ordering::Relaxed);

                loop {
                    // Wait until another thread is done or cancelled flushing
                    // and try flush it again just in case the flushing is cancelled
                    flush(&shared_data).await?;

                    prev_pending_requests =
                        pending_requests.fetch_sub(prev_pending_requests, Ordering::Relaxed);

                    if prev_pending_requests < max_pending_requests {
                        break;
                    }
                }
            }
        });

        let read_task = task::spawn(async move {
            let cancel_guard = read_end
                .get_shared_data()
                .get_auxiliary()
                .cancel_token
                .clone()
                .drop_guard();

            loop {
                let new_requests_submit = read_end.wait_for_new_request().await;
                if new_requests_submit == 0 {
                    // All responses is read in and there is no
                    // write_end/shared_data left.
                    cancel_guard.disarm();
                    break Ok::<_, Error>(());
                }

                // If attempt to read in more than new_requests_submit, then
                // `read_in_one_packet` might block forever.
                for _ in 0..new_requests_submit {
                    read_end.read_in_one_packet().await?;
                }
            }
        });

        Ok(Self {
            phantom_data: PhantomData,
            child,

            shared_data: write_end.into_shared_data(),
            read_task,
            flush_task,
        })
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        // Flush the data.
        //
        // Since there is no reference to `Sftp`, the only requests that
        // haven't yet flushed should be close requests.
        //
        // And there will not be any new requests.
        flush(&self.shared_data).await?;

        // Terminate flush_task, otherwise read_task would not return.
        self.flush_task.abort();
        match self.flush_task.await {
            Ok(res) => res?,
            Err(join_err) => {
                if !join_err.is_cancelled() {
                    return Err(join_err.into());
                }
            }
        }

        // Drop the shared_data, otherwise read_task would not return.
        debug_assert_eq!(self.shared_data.strong_count(), 2);
        drop(self.shared_data);

        // Wait for responses for all requests buffered and sent.
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

    fn write_end(&self) -> WriteEndWithCachedId<'_> {
        WriteEndWithCachedId::new(self, self.shared_data.clone())
    }

    /// Get maximum amount of bytes that one single write requests
    /// can write.
    pub fn max_write_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().write_len
    }

    /// Get maximum amount of bytes that one single read requests
    /// can read.
    pub fn max_read_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().read_len
    }

    /// Return a new [`OpenOptions`] object.
    pub fn options(&self) -> OpenOptions<'_> {
        OpenOptions::new(self)
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist, and will truncate
    /// it if it does.
    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    /// Attempts to open a file in read-only mode.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options().read(true).open(path).await
    }

    /// * `cwd` - The current working dir for the [`Fs`].
    ///           If `cwd` is empty, then it is set to use
    ///           the default directory set by the remote
    ///           `sftp-server`.
    pub fn fs(&self, cwd: impl Into<PathBuf>) -> Fs<'_> {
        Fs::new(self.write_end(), cwd.into())
    }

    fn auxiliary(&self) -> &Auxiliary {
        self.shared_data.get_auxiliary()
    }

    /// without doing anything and return `false`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn try_flush(&self) -> Result<bool, io::Error> {
        let auxiliary = self.auxiliary();

        let prev_pending_requests = auxiliary.pending_requests.load(Ordering::Relaxed);

        if self.shared_data.try_flush().await? {
            auxiliary.consume_pending_requests(prev_pending_requests);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Forcibly flush the write buffer.
    ///
    /// If another thread is doing flushing, then this function would
    /// wait until it completes or cancelled the future.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn flush(&self) -> Result<(), io::Error> {
        let auxiliary = self.auxiliary();

        let prev_pending_requests = auxiliary.pending_requests.load(Ordering::Relaxed);
        self.shared_data.flush().await?;
        auxiliary.consume_pending_requests(prev_pending_requests);

        Ok(())
    }

    /// Trigger flushing in the `flush_task`.
    ///
    /// If there are pending requests, then flushing would happen immediately.
    ///
    /// If not, then the next time a request is queued in the write buffer, it
    /// will be immediately flushed.
    pub fn trigger_flushing(&self) {
        self.auxiliary().flush_immediately.notify_one();
    }

    /// Return number of pending requests in the write buffer.
    pub fn get_pending_requests(&self) -> usize {
        self.auxiliary().pending_requests.load(Ordering::Relaxed)
    }

    /// Return a cancellation token that will be cancelled if the `flush_task`
    /// or `read_task` failed or when `sftp::Sftp::close` is called.
    ///
    /// Cancelling this returned token has no effect on any function in this
    /// module.
    pub fn get_cancellation_token(&self) -> CancellationToken {
        self.auxiliary().cancel_token.child_token()
    }
}
