use super::{Auxiliary, Error, Id, IdCacher, Sftp, WriteEnd};

use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;

use openssh_sftp_client::Error as SftpError;

/// A struct used to perform operations on remote filesystem.
#[derive(Debug)]
pub struct Fs<'s> {
    phantom_data: PhantomData<&'s Sftp<'s>>,

    write_end: WriteEnd,
    cwd: PathBuf,
    id: Option<Id>,
}

impl Clone for Fs<'_> {
    fn clone(&self) -> Self {
        Self::new(self.write_end.clone(), self.cwd.clone())
    }
}

impl Drop for Fs<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.cache_id(id);
        }
    }
}

impl Fs<'_> {
    pub(super) fn new(write_end: WriteEnd, cwd: PathBuf) -> Self {
        Self {
            phantom_data: PhantomData,

            write_end,
            cwd,

            id: None,
        }
    }

    fn get_auxiliary(&self) -> &Auxiliary {
        self.write_end.get_auxiliary()
    }

    fn get_id_mut(&mut self) -> Id {
        self.id
            .take()
            .unwrap_or_else(|| self.write_end.get_thread_local_cached_id())
    }

    fn cache_id(&self, id: Id) {
        self.write_end.cache_id(id);
    }

    fn cache_id_mut(&mut self, id: Id) {
        if self.id.is_none() {
            self.id = Some(id);
        } else {
            self.cache_id(id);
        }
    }

    async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, SftpError>,
        F: Future<Output = Result<(Id, R), SftpError>> + 'static,
    {
        let id = self.get_id_mut();

        let future = f(&mut self.write_end, id)?;

        let (id, ret) = self.get_auxiliary().cancel_if_task_failed(future).await?;

        self.cache_id_mut(id);

        Ok(ret)
    }
}
