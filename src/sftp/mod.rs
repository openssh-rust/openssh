use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use bytes::BytesMut;
use openssh_sftp_client::{connect_with_auxiliary, Error as SftpError};
use tokio::{task, time};

pub use openssh_sftp_client::{FileType, Permissions, UnixTimeStamp};

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
pub use metadata::{MetaData, MetaDataBuilder};

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
        let limits = auxiliary::Limits {
            read_len: read_len.try_into().unwrap_or(u32::MAX - 300),
            write_len: write_len.try_into().unwrap_or(u32::MAX - 300),
        };

        let auxiliary = write_end.get_auxiliary();

        *auxiliary.conn_info.write() = auxiliary::ConnInfo { limits, extensions };
        auxiliary.thread_local_cache.get_or(|| Cache::new(Some(id)));

        let shared_data = SharedData::clone(&write_end);
        let flush_interval = options.get_flush_interval();
        let flush_task = task::spawn(async move {
            let mut interval = time::interval(flush_interval);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

            let auxiliary = shared_data.get_auxiliary();
            let flush_end_notify = &auxiliary.flush_end_notify;

            let _cancel_guard = auxiliary.cancel_token.clone().drop_guard();

            // The loop can only return `Err`
            loop {
                flush_end_notify.notified().await;
                interval.tick().await;

                // Wait until another thread is done or cancelled flushing
                // and try flush it again just in case the flushing is cancelled
                flush(&shared_data).await?;
            }
        });

        let read_task = task::spawn(async move {
            let mut read_end = read_end;

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
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
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

    fn write_end(&self) -> WriteEndWithCachedId {
        WriteEnd::new(self.shared_data.clone()).into()
    }

    /// Return a new [`OpenOptions`] object.
    pub fn options(&self) -> OpenOptions<'_> {
        OpenOptions::new(self)
    }

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options().read(true).open(path).await
    }

    /// * `cwd` - The current working dir for the [`Fs`].
    ///           If `cwd` is `None`, then it is set to `~`.
    pub fn fs(&self, cwd: Option<impl Into<PathBuf>>) -> Fs<'_> {
        Fs::new(
            self,
            self.write_end(),
            cwd.map(Into::into).unwrap_or_else(|| "~".into()),
        )
    }

    /// Forcibly flush the write buffer.
    ///
    /// If another thread is doing flushing, then this function would return
    /// without doing anything and return `false`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn try_flush(&self) -> Result<bool, io::Error> {
        Ok(self.shared_data.try_flush().await?)
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
        self.shared_data.flush().await?;

        Ok(())
    }
}
