use openssh_sftp_client::NameEntry;

/// Read dir
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct ReadDir(pub(super) Box<[NameEntry]>);

/// Dir entry
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct DirEntry(NameEntry);
