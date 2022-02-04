use super::super::{FileType, MetaData};

use openssh_sftp_client::NameEntry;
use std::path::Path;

/// Dir entry
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct DirEntry(NameEntry);

impl DirEntry {
    /// Return filename of the dir entry.
    pub fn filename(&self) -> &Path {
        &self.0.filename
    }

    /// Return filename of the dir entry as a mutable reference.
    pub fn filename_mut(&mut self) -> &mut Box<Path> {
        &mut self.0.filename
    }

    /// Return metadata for the dir entry.
    pub fn metadata(&self) -> MetaData {
        MetaData::new(self.0.attrs)
    }

    /// Return the file type for the dir entry.
    pub fn file_type(&self) -> Option<FileType> {
        self.metadata().file_type()
    }
}

/// Read dir
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct ReadDir(pub(super) Box<[NameEntry]>);
