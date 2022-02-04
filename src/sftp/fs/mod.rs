use super::{Auxiliary, Error, Id, OwnedHandle, Sftp, WriteEnd, WriteEndWithCachedId};

use std::borrow::Cow;
use std::future::Future;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use openssh_sftp_client::{Error as SftpError, FileAttrs, Permissions};

mod dir;
pub use dir::{DirEntry, ReadDir};

/// A struct used to perform operations on remote filesystem.
#[derive(Debug, Clone)]
pub struct Fs<'s> {
    phantom_data: PhantomData<&'s Sftp<'s>>,

    write_end: WriteEndWithCachedId,
    cwd: Box<Path>,
}

impl<'s> Fs<'s> {
    pub(super) fn new(write_end: WriteEndWithCachedId, cwd: PathBuf) -> Self {
        Self {
            phantom_data: PhantomData,

            write_end,
            cwd: cwd.into_boxed_path(),
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
        self.cwd = cwd.into().into_boxed_path();
    }

    fn concat_path_if_needed<'path>(&self, path: &'path Path) -> Cow<'path, Path> {
        if path.is_absolute() {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(self.cwd.join(path))
        }
    }

    async fn open_dir_impl(&mut self, path: &Path) -> Result<Dir<'_>, Error> {
        let path = self.concat_path_if_needed(path);

        self.send_request(|write_end, id| Ok(write_end.send_opendir_request(id, path)?.wait()))
            .await
            .map(|handle| Dir(OwnedHandle::new(self.write_end.clone(), handle)))
    }

    /// Open a remote dir
    pub async fn open_dir(&mut self, path: impl AsRef<Path>) -> Result<Dir<'_>, Error> {
        self.open_dir_impl(path.as_ref()).await
    }

    /// Create a directory builder.
    pub fn dir_builder(&mut self) -> DirBuilder<'_, 's> {
        DirBuilder {
            fs: self,
            attrs: FileAttrs::new(),
        }
    }

    /// Creates a new, empty directory at the provided path.
    pub async fn create_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.dir_builder().create(path).await
    }

    async fn remove_dir_impl(&mut self, path: &Path) -> Result<(), Error> {
        let path = self.concat_path_if_needed(path);

        self.send_request(|write_end, id| Ok(write_end.send_rmdir_request(id, path)?.wait()))
            .await
    }

    /// Removes an existing, empty directory.
    pub async fn remove_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.remove_dir_impl(path.as_ref()).await
    }
}

/// Remote Directory
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Dir<'s>(OwnedHandle<'s>);

impl Dir<'_> {
    /// Read dir.
    pub async fn read_dir(&mut self) -> Result<ReadDir, Error> {
        self.0
            .send_request(|write_end, handle, id| {
                Ok(write_end.send_readdir_request(id, handle)?.wait())
            })
            .await
            .map(ReadDir::new)
    }

    /// Close dir.
    pub async fn close(self) -> Result<(), Error> {
        self.0.close().await
    }
}

/// Builder for new directory to create.
#[derive(Debug)]
pub struct DirBuilder<'a, 's> {
    fs: &'a mut Fs<'s>,
    attrs: FileAttrs,
}

impl DirBuilder<'_, '_> {
    /// Reset builder back to default.
    pub fn reset(&mut self) -> &mut Self {
        self.attrs = FileAttrs::new();
        self
    }

    /// Set id of the dir to be built.
    pub fn id(&mut self, (uid, gid): (u32, u32)) -> &mut Self {
        self.attrs.set_id(uid, gid);
        self
    }

    /// Set permissions of the dir to be built.
    pub fn permissions(&mut self, perm: Permissions) -> &mut Self {
        self.attrs.set_permissions(perm);
        self
    }

    async fn create_impl(&mut self, path: &Path) -> Result<(), Error> {
        let fs = &mut self.fs;

        let path = fs.concat_path_if_needed(path);

        fs.send_request(|write_end, id| {
            Ok(write_end.send_mkdir_request(id, path, self.attrs)?.wait())
        })
        .await
    }

    /// Creates the specified directory with the configured options.
    pub async fn create(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.create_impl(path.as_ref()).await
    }
}
