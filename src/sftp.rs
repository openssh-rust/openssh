use super::{child::RemoteChildImp, ChildStdin, ChildStdout, Error, Session};

use std::process::ExitStatus;

use openssh_sftp_client::{connect, Extensions, Limits, WriteEnd};
use tokio::task;

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    session: &'s Session,
    child: RemoteChildImp,

    write_end: WriteEnd<Vec<u8>>,
    read_task: task::JoinHandle<Result<(), Error>>,

    extensions: Extensions,
    limits: Limits,
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

                // If attempt to read in more than new_requests_submit, then
                // `read_in_one_packet` might block forever.
                for _ in 0..new_requests_submit {
                    read_end.read_in_one_packet().await?;
                }
            }
        });

        let id = write_end.create_response_id();

        let limits = if extensions.limits {
            let awaitable = write_end.send_limits_request(id)?;
            write_end.flush().await?;
            awaitable.wait().await?.1
        } else {
            Limits {
                packet_len: 0,
                read_len: openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64,
                write_len: openssh_sftp_client::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64,
                open_handles: 0,
            }
        };

        Ok(Self {
            session,
            child,

            write_end,
            read_task,

            extensions,
            limits,
        })
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
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
}
