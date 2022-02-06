use super::{
    Auxiliary, Buffer, Error, Id, MetaData, MetaDataBuilder, OwnedHandle, Permissions, Sftp,
    SftpError, WriteEnd, WriteEndWithCachedId,
};

use std::borrow::Cow;
use std::cmp::min;
use std::path::{Path, PathBuf};

use bytes::BytesMut;

mod dir;
pub use dir::{DirEntry, ReadDir};

type AwaitableStatus = openssh_sftp_client::AwaitableStatus<Buffer>;
type AwaitableAttrs = openssh_sftp_client::AwaitableAttrs<Buffer>;
type SendLinkingRequest =
    fn(&mut WriteEnd, Id, Cow<'_, Path>, Cow<'_, Path>) -> Result<AwaitableStatus, SftpError>;

/// A struct used to perform operations on remote filesystem.
#[derive(Debug, Clone)]
pub struct Fs<'s> {
    sftp: &'s Sftp<'s>,

    write_end: WriteEndWithCachedId,
    cwd: Box<Path>,
}

impl<'s> Fs<'s> {
    pub(super) fn new(sftp: &'s Sftp<'s>, write_end: WriteEndWithCachedId, cwd: PathBuf) -> Self {
        Self {
            sftp,

            write_end,
            cwd: cwd.into_boxed_path(),
        }
    }

    fn get_auxiliary(&self) -> &Auxiliary {
        self.write_end.get_auxiliary()
    }

    /// Return current working dir.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Set current working dir.
    ///
    /// * `cwd` - Can include `~`.
    ///           If it is empty, then it is set to use the default
    ///           directory set by the remote `sftp-server`.
    pub fn set_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.cwd = cwd.into().into_boxed_path();
    }

    fn concat_path_if_needed<'path>(&self, path: &'path Path) -> Cow<'path, Path> {
        if path.is_absolute() || self.cwd.as_os_str().is_empty() {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(self.cwd.join(path))
        }
    }

    async fn open_dir_impl(&mut self, path: &Path) -> Result<Dir<'_>, Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(write_end.send_opendir_request(id, path)?.wait()))
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
            metadata_builder: MetaDataBuilder::new(),
        }
    }

    /// Creates a new, empty directory at the provided path.
    pub async fn create_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.dir_builder().create(path).await
    }

    async fn remove_impl(
        &mut self,
        path: &Path,
        f: fn(&mut WriteEnd, Id, Cow<'_, Path>) -> Result<AwaitableStatus, SftpError>,
    ) -> Result<(), Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
            .await
    }

    /// Removes an existing, empty directory.
    pub async fn remove_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.remove_impl(path.as_ref(), WriteEnd::send_rmdir_request)
            .await
    }

    /// Removes a file from remote filesystem.
    pub async fn remove_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.remove_impl(path.as_ref(), WriteEnd::send_remove_request)
            .await
    }

    async fn canonicalize_impl(&mut self, path: &Path) -> Result<PathBuf, Error> {
        let path = self.concat_path_if_needed(path);

        let f = if self.get_auxiliary().extensions().expand_path {
            // This supports canonicalisation of relative paths and those that
            // need tilde-expansion, i.e. “~”, “~/…” and “~user/…”.
            //
            // These paths are expanded using shell-like rules and the resultant
            // path is canonicalised similarly to WriteEnd::send_realpath_request.
            WriteEnd::send_expand_path_request
        } else {
            WriteEnd::send_realpath_request
        };

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
            .await
            .map(Into::into)
    }

    /// Returns the canonical, absolute form of a path with all intermediate
    /// components normalized and symbolic links resolved.
    pub async fn canonicalize(&mut self, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
        self.canonicalize_impl(path.as_ref()).await
    }

    async fn linking_impl(
        &mut self,
        src: &Path,
        dst: &Path,
        f: SendLinkingRequest,
    ) -> Result<(), Error> {
        let src = self.concat_path_if_needed(src);
        let dst = self.concat_path_if_needed(dst);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, src, dst)?.wait()))
            .await
    }

    async fn hard_link_impl(&mut self, src: &Path, dst: &Path) -> Result<(), Error> {
        if !self.get_auxiliary().extensions().hardlink {
            return Err(SftpError::UnsupportedExtension(&"hardlink").into());
        }

        self.linking_impl(src, dst, WriteEnd::send_hardlink_request)
            .await
    }

    /// Creates a new hard link on the remote filesystem.
    pub async fn hard_link(
        &mut self,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> Result<(), Error> {
        self.hard_link_impl(src.as_ref(), dst.as_ref()).await
    }

    /// Creates a new symlink on the remote filesystem.
    pub async fn symlink(
        &mut self,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> Result<(), Error> {
        self.linking_impl(src.as_ref(), dst.as_ref(), WriteEnd::send_symlink_request)
            .await
    }

    async fn rename_impl(&mut self, from: &Path, to: &Path) -> Result<(), Error> {
        let f = if self.get_auxiliary().extensions().posix_rename {
            // posix rename is guaranteed to be atomic
            WriteEnd::send_posix_rename_request
        } else {
            WriteEnd::send_rename_request
        };

        self.linking_impl(from, to, f).await
    }

    /// Renames a file or directory to a new name, replacing the original file if to already exists.
    ///
    /// This will not work if the new name is on a different mount point.
    pub async fn rename(
        &mut self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> Result<(), Error> {
        self.rename_impl(from.as_ref(), to.as_ref()).await
    }

    async fn read_link_impl(&mut self, path: &Path) -> Result<PathBuf, Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(write_end.send_readlink_request(id, path)?.wait()))
            .await
            .map(Into::into)
    }

    /// Reads a symbolic link, returning the file that the link points to.
    pub async fn read_link(&mut self, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
        self.read_link_impl(path.as_ref()).await
    }

    async fn set_metadata_impl(&mut self, path: &Path, metadata: MetaData) -> Result<(), Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| {
                Ok(write_end
                    .send_setstat_request(id, path, metadata.into_inner())?
                    .wait())
            })
            .await
    }

    /// Change the metadata of a file or a directory.
    pub async fn set_metadata(
        &mut self,
        path: impl AsRef<Path>,
        metadata: MetaData,
    ) -> Result<(), Error> {
        self.set_metadata_impl(path.as_ref(), metadata).await
    }

    async fn set_permissions_impl(&mut self, path: &Path, perm: Permissions) -> Result<(), Error> {
        self.set_metadata_impl(path, MetaDataBuilder::new().permissions(perm).create())
            .await
    }

    /// Changes the permissions found on a file or a directory.
    pub async fn set_permissions(
        &mut self,
        path: impl AsRef<Path>,
        perm: Permissions,
    ) -> Result<(), Error> {
        self.set_permissions_impl(path.as_ref(), perm).await
    }

    async fn metadata_impl(
        &mut self,
        path: &Path,
        f: fn(&mut WriteEnd, Id, Cow<'_, Path>) -> Result<AwaitableAttrs, SftpError>,
    ) -> Result<MetaData, Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
            .await
            .map(MetaData::new)
    }

    /// Given a path, queries the file system to get information about a file,
    /// directory, etc.
    pub async fn metadata(&mut self, path: impl AsRef<Path>) -> Result<MetaData, Error> {
        self.metadata_impl(path.as_ref(), WriteEnd::send_stat_request)
            .await
    }

    /// Queries the file system metadata for a path.
    pub async fn symlink_metadata(&mut self, path: impl AsRef<Path>) -> Result<MetaData, Error> {
        self.metadata_impl(path.as_ref(), WriteEnd::send_lstat_request)
            .await
    }

    async fn read_impl(&mut self, path: &Path) -> Result<BytesMut, Error> {
        let path = self.concat_path_if_needed(path);

        let mut file = self.sftp.open(path).await?;
        let max_read_len = file.max_read_len();

        let cap_to_reserve: usize = if let Some(len) = file.metadata().await?.len() {
            // To detect EOF, we need to a little bit more then the length
            // of the file.
            len.saturating_add(300)
                .try_into()
                .unwrap_or(max_read_len as usize)
        } else {
            max_read_len as usize
        };

        let mut buffer = BytesMut::with_capacity(cap_to_reserve);

        loop {
            let cnt = buffer.len();

            let n: u32 = if cnt <= cap_to_reserve {
                // To detect EOF, we need to a little bit more then the
                // length of the file.
                (cap_to_reserve - cnt)
                    .saturating_add(300)
                    .try_into()
                    .map(|n| min(n, max_read_len))
                    .unwrap_or(max_read_len)
            } else {
                max_read_len
            };
            buffer.reserve(n.try_into().unwrap_or(usize::MAX));

            if let Some(bytes) = file.read(n, buffer.split_off(cnt)).await? {
                buffer.unsplit(bytes);
            } else {
                // Eof
                break Ok(buffer);
            }
        }
    }

    /// Reads the entire contents of a file into a bytes.
    pub async fn read(&mut self, path: impl AsRef<Path>) -> Result<BytesMut, Error> {
        self.read_impl(path.as_ref()).await
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
    metadata_builder: MetaDataBuilder,
}

impl DirBuilder<'_, '_> {
    /// Reset builder back to default.
    pub fn reset(&mut self) -> &mut Self {
        self.metadata_builder = MetaDataBuilder::new();
        self
    }

    /// Set id of the dir to be built.
    pub fn id(&mut self, (uid, gid): (u32, u32)) -> &mut Self {
        self.metadata_builder.id((uid, gid));
        self
    }

    /// Set permissions of the dir to be built.
    pub fn permissions(&mut self, perm: Permissions) -> &mut Self {
        self.metadata_builder.permissions(perm);
        self
    }

    async fn create_impl(&mut self, path: &Path) -> Result<(), Error> {
        let fs = &mut self.fs;

        let path = fs.concat_path_if_needed(path);
        let attrs = self.metadata_builder.create().into_inner();

        fs.write_end
            .send_request(|write_end, id| Ok(write_end.send_mkdir_request(id, path, attrs)?.wait()))
            .await
    }

    /// Creates the specified directory with the configured options.
    pub async fn create(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.create_impl(path.as_ref()).await
    }
}
