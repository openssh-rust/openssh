use super::UnixTimeStamp;

use openssh_sftp_client::{FileAttrs, FileType as SftpFileType, Permissions as SftpPermissions};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};

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
        self.0.set_permissions(perm.0);
        self
    }

    /// Set size of the metadata to built.
    pub fn len(&mut self, len: u64) -> &mut Self {
        self.0.set_size(len);
        self
    }

    /// Create a [`MetaData`].
    pub fn create(&self) -> MetaData {
        MetaData::new(self.0)
    }
}

/// Metadata information about a file.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
        self.0.get_permissions().map(Permissions)
    }

    /// Returns the file type.
    ///
    /// Return `None` if the server did not return
    /// the file type.
    pub fn file_type(&self) -> Option<FileType> {
        self.0.get_filetype().map(FileType)
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

/// A structure representing a type of file with accessors for each file type.
/// It is returned by [`MetaData::file_type`] method.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct FileType(SftpFileType);

impl FileType {
    /// Tests whether this file type represents a directory.
    pub fn is_dir(&self) -> bool {
        self.0 == SftpFileType::Directory
    }

    /// Tests whether this file type represents a regular file.
    pub fn is_file(&self) -> bool {
        self.0 == SftpFileType::RegularFile
    }

    /// Tests whether this file type represents a symbolic link.
    pub fn is_symlink(&self) -> bool {
        self.0 == SftpFileType::Symlink
    }
}

/// Implement [`FileTypeExt`] just to be compatible with [`std::fs::FileType`].
impl FileTypeExt for FileType {
    fn is_fifo(&self) -> bool {
        self.0 == SftpFileType::FIFO
    }

    /// Tests whether this file type represents a socket.
    fn is_socket(&self) -> bool {
        self.0 == SftpFileType::Socket
    }

    fn is_block_device(&self) -> bool {
        self.0 == SftpFileType::BlockDevice
    }

    fn is_char_device(&self) -> bool {
        self.0 == SftpFileType::CharacterDevice
    }
}

/// Representation of the various permissions on a file.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Permissions(SftpPermissions);

macro_rules! impl_getter_setter {
    ($getter_name:ident, $setter_name:ident, $variant:ident, $variant_name:expr) => {
        #[doc = "Tests whether "]
        #[doc = $variant_name]
        #[doc = " bit is set."]
        pub fn $getter_name(&self) -> bool {
            self.0.intersects(SftpPermissions::$variant)
        }

        #[doc = "Modify the "]
        #[doc = $variant_name]
        #[doc = " bit."]
        pub fn $setter_name(&mut self, value: bool) {
            self.0.set(SftpPermissions::$variant, value);
        }
    };
}

impl Permissions {
    /// Create a new permissions object with zero permissions
    /// set.
    pub const fn new() -> Self {
        Self(SftpPermissions::empty())
    }

    impl_getter_setter!(suid, set_suid, SET_UID, "set-user-id");
    impl_getter_setter!(sgid, set_sgid, SET_GID, "set-group-id");
    impl_getter_setter!(svtx, set_vtx, SET_VTX, "set-sticky-bit");

    impl_getter_setter!(
        read_by_owner,
        set_read_by_owner,
        READ_BY_OWNER,
        "read by owner"
    );
    impl_getter_setter!(
        write_by_owner,
        set_write_by_owner,
        WRITE_BY_OWNER,
        "write by owner"
    );
    impl_getter_setter!(
        execute_by_owner,
        set_execute_by_owner,
        EXECUTE_BY_OWNER,
        "execute by owner"
    );

    impl_getter_setter!(
        read_by_group,
        set_read_by_group,
        READ_BY_GROUP,
        "read by group"
    );
    impl_getter_setter!(
        write_by_group,
        set_write_by_group,
        WRITE_BY_GROUP,
        "write by group"
    );
    impl_getter_setter!(
        execute_by_group,
        set_execute_by_group,
        EXECUTE_BY_GROUP,
        "execute by group"
    );

    impl_getter_setter!(
        read_by_other,
        set_read_by_other,
        READ_BY_OTHER,
        "read by other"
    );
    impl_getter_setter!(
        write_by_other,
        set_write_by_other,
        WRITE_BY_OTHER,
        "write by other"
    );
    impl_getter_setter!(
        execute_by_other,
        set_execute_by_other,
        EXECUTE_BY_OTHER,
        "execute by other"
    );

    /// Returns `true` if these permissions describe an unwritable file
    /// that no one can write to.
    pub fn readonly(&self) -> bool {
        !self.write_by_owner() && !self.write_by_group() && !self.write_by_other()
    }

    /// Modifies the readonly flag for this set of permissions.
    ///
    /// If the readonly argument is true, it will remove write permissions
    /// from all parties.
    ///
    /// Conversely, if it’s false, it will permit writing from all parties.
    ///
    /// This operation does not modify the filesystem.
    ///
    /// To modify the filesystem use the [`super::Fs::set_permissions`] or
    /// the [`super::File::set_permissions`] function.
    pub fn set_readonly(&mut self, readonly: bool) {
        let writable = !readonly;

        self.set_write_by_owner(writable);
        self.set_write_by_group(writable);
        self.set_write_by_other(writable);
    }
}

impl PermissionsExt for Permissions {
    fn mode(&self) -> u32 {
        self.0.bits()
    }

    fn set_mode(&mut self, mode: u32) {
        self.0 = SftpPermissions::from_bits_truncate(mode);
    }

    fn from_mode(mode: u32) -> Self {
        Self(SftpPermissions::from_bits_truncate(mode))
    }
}
