use super::child::{delegate, RemoteChildImp};
use super::{ChildStdin, ChildStdout, Error, Session};

use openssh_sftp_client::highlevel;
use std::marker::PhantomData;
use std::ops::Deref;
use std::process::ExitStatus;

pub use highlevel::{
    Dir, DirBuilder, DirEntry, File, FileType, Fs, MetaData, MetaDataBuilder, OpenOptions,
    Permissions, ReadDir, SftpOptions, TokioCompactFile, UnixTimeStamp, UnixTimeStampError,
};

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp<'s> {
    phantom_data: PhantomData<&'s Session>,
    child: RemoteChildImp,

    inner: highlevel::Sftp,
}

impl<'s> Sftp<'s> {
    pub(crate) async fn new(
        child: RemoteChildImp,
        stdin: ChildStdin,
        stdout: ChildStdout,
        options: SftpOptions,
    ) -> Result<Sftp<'s>, Error> {
        Ok(Self {
            phantom_data: PhantomData,
            child,

            inner: highlevel::Sftp::new(stdin, stdout, options).await?,
        })
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        self.inner.close().await?;

        let res: Result<ExitStatus, Error> = delegate!(self.child, child, { child.wait().await });
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

impl Deref for Sftp<'_> {
    type Target = highlevel::Sftp;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
