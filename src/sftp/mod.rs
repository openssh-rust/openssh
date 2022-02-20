use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::cmp::min;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::Ordering;

use bytes::BytesMut;
use derive_destructure2::destructure;
use openssh_sftp_client::{connect_with_auxiliary, Error as SftpError};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use openssh_sftp_client::{Extensions, UnixTimeStamp, UnixTimeStampError};

mod cancel_utility;
use cancel_utility::BoxedWaitForCancellationFuture;

mod options;
pub use options::SftpOptions;

mod tasks;
use tasks::{create_flush_task, create_read_task};

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
type ReadEnd = openssh_sftp_client::ReadEnd<Buffer, Auxiliary>;
type SharedData = openssh_sftp_client::SharedData<Buffer, Auxiliary>;
type Id = openssh_sftp_client::Id<Buffer>;
type Data = openssh_sftp_client::Data<Buffer>;

async fn flush(shared_data: &SharedData) -> Result<(), Error> {
    Ok(shared_data.flush().await.map_err(SftpError::from)?)
}

/// A file-oriented channel to a remote host.
#[derive(Debug, destructure)]
pub struct Sftp<'s> {
    phantom_data: PhantomData<&'s Session>,
    child: RemoteChildImp,

    shared_data: SharedData,
    flush_task: JoinHandle<Result<(), Error>>,
    read_task: JoinHandle<Result<(), Error>>,
}

impl<'s> Sftp<'s> {
    async fn set_limits(
        &self,
        write_end: WriteEnd,
        options: SftpOptions,
        extensions: Extensions,
    ) -> Result<(), Error> {
        let mut write_end = WriteEndWithCachedId::new(self, write_end);

        let default_download_buflen =
            openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64;
        let default_upload_buflen =
            openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64;

        // sftp can accept packet as large as u32::MAX, the header itself
        // is at least 9 bytes long.
        let default_max_packet_len = u32::MAX - 9;

        let (read_len, write_len, packet_len) = if extensions.limits {
            let mut limits = write_end
                .send_request(|write_end, id| Ok(write_end.send_limits_request(id)?.wait()))
                .await?;

            if limits.read_len == 0 {
                limits.read_len = default_download_buflen;
            }

            if limits.write_len == 0 {
                limits.write_len = default_upload_buflen;
            }

            (
                limits.read_len,
                limits.write_len,
                limits
                    .packet_len
                    .try_into()
                    .unwrap_or(default_max_packet_len),
            )
        } else {
            (
                default_download_buflen,
                default_upload_buflen,
                default_max_packet_len,
            )
        };

        // Each read/write request also has a header and contains a handle,
        // which is 4-byte long for openssh but can be at most 256 bytes long
        // for other implementations.

        let read_len = read_len.try_into().unwrap_or(packet_len - 300);
        let read_len = options
            .get_max_read_len()
            .map(|v| min(v, read_len))
            .unwrap_or(read_len);

        let write_len = write_len.try_into().unwrap_or(packet_len - 300);
        let write_len = options
            .get_max_write_len()
            .map(|v| min(v, write_len))
            .unwrap_or(write_len);

        let limits = auxiliary::Limits {
            read_len,
            write_len,
        };

        write_end
            .get_auxiliary()
            .conn_info
            .set(auxiliary::ConnInfo { limits, extensions })
            .expect("auxiliary.conn_info shall be empty");

        Ok(())
    }

    pub(crate) async fn new(
        child: RemoteChildImp,
        stdin: ChildStdin,
        stdout: ChildStdout,
        options: SftpOptions,
    ) -> Result<Sftp<'s>, Error> {
        let (write_end, read_end, extensions) = connect_with_auxiliary(
            stdout,
            stdin,
            Auxiliary::new(options.get_max_pending_requests()),
        )
        .await?;

        // Create sftp here.
        //
        // It would also gracefully shutdown `flush_task` and `read_task` if
        // the future is cancelled or error is encounted.
        let sftp = Self {
            phantom_data: PhantomData,
            child,

            shared_data: SharedData::clone(&write_end),

            flush_task: create_flush_task(
                SharedData::clone(&write_end),
                options.get_flush_interval(),
            ),
            read_task: create_read_task(read_end),
        };

        sftp.set_limits(write_end, options, extensions).await?;

        Ok(sftp)
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        let (_phantom_data, child, shared_data, flush_task, read_task) = self.destructure();

        // This will terminate flush_task, otherwise read_task would not return.
        shared_data.get_auxiliary().requests_shutdown();

        flush_task.await??;

        // Drop the shared_data, otherwise read_task would not return.
        debug_assert_eq!(shared_data.strong_count(), 2);
        drop(shared_data);

        // Wait for responses for all requests buffered and sent.
        read_task.await??;

        let res: Result<ExitStatus, Error> =
            crate::child::delegate!(child, child, { child.wait().await });
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
        WriteEndWithCachedId::new(self, WriteEnd::new(self.shared_data.clone()))
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
    pub fn get_pending_requests(&self) -> u32 {
        self.auxiliary().pending_requests.load(Ordering::Relaxed)
    }

    /// Return a cancellation token that will be cancelled if the `flush_task`
    /// or `read_task` failed is called.
    ///
    /// Cancelling this returned token has no effect on any function in this
    /// module.
    pub fn get_cancellation_token(&self) -> CancellationToken {
        self.auxiliary().cancel_token.child_token()
    }
}

impl Drop for Sftp<'_> {
    fn drop(&mut self) {
        // This will terminate flush_task, otherwise read_task would not return.
        self.shared_data.get_auxiliary().requests_shutdown();
    }
}
