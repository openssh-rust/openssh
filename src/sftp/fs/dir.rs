use super::super::{FileType, MetaData};

use openssh_sftp_client::NameEntry;
use std::path::Path;
use std::slice::{Iter, IterMut};
use std::vec::IntoIter;

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
pub struct ReadDir(pub(super) Box<[DirEntry]>);

impl ReadDir {
    pub(super) fn new(entries: Box<[NameEntry]>) -> Self {
        let ptr = Box::into_raw(entries);

        // Safety: DirEntry is transparent
        ReadDir(unsafe { Box::from_raw(ptr as *mut [DirEntry]) })
    }

    /// Return slice of [`DirEntry`]s.
    pub fn as_slice(&self) -> &[DirEntry] {
        &self.0
    }

    /// Return mutable slice of [`DirEntry`]s.
    pub fn as_mut_slice(&mut self) -> &mut [DirEntry] {
        &mut self.0
    }

    /// Return boxed slice of [`DirEntry`]s.
    pub fn into_inner(self) -> Box<[DirEntry]> {
        self.0
    }
}

impl<'a> IntoIterator for &'a ReadDir {
    type Item = &'a DirEntry;
    type IntoIter = Iter<'a, DirEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a> IntoIterator for &'a mut ReadDir {
    type Item = &'a mut DirEntry;
    type IntoIter = IterMut<'a, DirEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

impl IntoIterator for ReadDir {
    type Item = DirEntry;
    type IntoIter = IntoIter<DirEntry>;

    fn into_iter(self) -> Self::IntoIter {
        let vec: Vec<DirEntry> = self.into_inner().into();
        vec.into_iter()
    }
}
