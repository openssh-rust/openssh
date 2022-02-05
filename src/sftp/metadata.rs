use super::{FileType, Permissions, UnixTimeStamp};

use openssh_sftp_client::FileAttrs;

/// Builder of [`MetaData`].
#[derive(Debug, Default, Copy, Clone)]
pub struct MetaDataBuilder(FileAttrs);

impl MetaDataBuilder {
    /// Create a builder.
    pub const fn new() -> Self {
        Self(FileAttrs::new())
    }

    /// Reset builder back to default.
    pub fn reset(&mut self) -> &mut Self {
        self.0 = FileAttrs::new();
        self
    }

    /// Set id of the metadata to be built.
    pub fn id(&mut self, (uid, gid): (u32, u32)) -> &mut Self {
        self.0.set_id(uid, gid);
        self
    }

    /// Set permissions of the metadata to be built.
    pub fn permissions(&mut self, perm: Permissions) -> &mut Self {
        self.0.set_permissions(perm);
        self
    }

    /// Set size of the metadata to built.
    pub fn size(&mut self, size: u64) -> &mut Self {
        self.0.set_size(size);
        self
    }

    /// Create a [`MetaData`].
    pub fn create(&self) -> MetaData {
        MetaData::new(self.0)
    }
}

/// Metadata information about a file.
#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub struct MetaData(FileAttrs);

#[allow(clippy::len_without_is_empty)]
impl MetaData {
    pub(super) fn new(attrs: FileAttrs) -> Self {
        Self(attrs)
    }

    pub(super) fn into_inner(self) -> FileAttrs {
        self.0
    }

    /// Returns the size of the file in bytes.
    ///
    /// Return `None` if the server did not return
    /// the size.
    pub fn len(&self) -> Option<u64> {
        self.0.get_size()
    }

    /// Returns the user ID of the owner.
    ///
    /// Return `None` if the server did not return
    /// the uid.
    pub fn uid(&self) -> Option<u32> {
        self.0.get_id().map(|(uid, _gid)| uid)
    }

    /// Returns the group ID of the owner.
    ///
    /// Return `None` if the server did not return
    /// the gid.
    pub fn gid(&self) -> Option<u32> {
        self.0.get_id().map(|(_uid, gid)| gid)
    }

    /// Returns the permissions.
    ///
    /// Return `None` if the server did not return
    /// the permissions.
    pub fn permissions(&self) -> Option<Permissions> {
        self.0.get_permissions()
    }

    /// Returns the file type.
    ///
    /// Return `None` if the server did not return
    /// the file type.
    pub fn file_type(&self) -> Option<FileType> {
        self.0.get_filetype()
    }

    /// Returns the last access time.
    ///
    /// Return `None` if the server did not return
    /// the last access time.
    pub fn accessed(&self) -> Option<UnixTimeStamp> {
        self.0.get_time().map(|(atime, _mtime)| atime)
    }

    /// Returns the last modification time.
    ///
    /// Return `None` if the server did not return
    /// the last modification time.
    pub fn modified(&self) -> Option<UnixTimeStamp> {
        self.0.get_time().map(|(_atime, mtime)| mtime)
    }
}
