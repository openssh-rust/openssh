use super::{Auxiliary, Error, Id, Sftp, WriteEnd, WriteEndWithCachedId};

use std::future::Future;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use openssh_sftp_client::Error as SftpError;

/// A struct used to perform operations on remote filesystem.
#[derive(Debug, Clone)]
pub struct Fs<'s> {
    phantom_data: PhantomData<&'s Sftp<'s>>,

    write_end: WriteEndWithCachedId,
    cwd: PathBuf,
}

impl Fs<'_> {
    pub(super) fn new(write_end: WriteEndWithCachedId, cwd: PathBuf) -> Self {
        Self {
            phantom_data: PhantomData,

            write_end,
            cwd,
        }
    }

    fn get_auxiliary(&self) -> &Auxiliary {
        self.write_end.get_auxiliary()
    }

    async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        let id = self.write_end.get_id_mut();

        let future = f(&mut self.write_end, id)?;

        let (id, ret) = self.get_auxiliary().cancel_if_task_failed(future).await?;

        self.write_end.cache_id_mut(id);

        Ok(ret)
    }

    /// Return current working dir.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Set current working dir.
    ///
    /// * `cwd` - Can include `~`.
    pub fn set_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.cwd = cwd.into();
    }
}
