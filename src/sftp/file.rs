use super::{Error, Id, Sftp, WriteEnd};

use std::borrow::Cow;
use std::path::Path;

use openssh_sftp_client::{CreateFlags, FileAttrs, HandleOwned};

#[derive(Debug)]
pub struct OpenOptions<'sftp, 's> {
    sftp: &'sftp Sftp<'s>,
    options: openssh_sftp_client::OpenOptions,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl<'sftp, 's> OpenOptions<'sftp, 's> {
    pub(super) fn new(sftp: &'sftp Sftp<'s>) -> Self {
        Self {
            sftp,
            options: openssh_sftp_client::OpenOptions::new(),
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    pub fn read(&mut self, read: bool) -> &mut Self {
        self.options = self.options.read(read);
        self
    }

    pub fn write(&mut self, write: bool) -> &mut Self {
        self.options = self.options.write(write);
        self
    }

    pub fn append(&mut self, append: bool) -> &mut Self {
        self.options = self.options.append(append);
        self
    }

    /// Only take effect if [`OpenOptions::create`] is set to `true`.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'_, '_>, Error> {
        let filename = Cow::Borrowed(path.as_ref());

        let params = if self.create {
            let flags = if self.create_new {
                CreateFlags::Excl
            } else if self.truncate {
                CreateFlags::Trunc
            } else {
                CreateFlags::None
            };

            self.options.create(filename, flags, FileAttrs::new())
        } else {
            self.options.open(filename)
        };

        let sftp = self.sftp;
        let mut write_end = sftp.write_end();
        let id = sftp.get_thread_local_cached_id();

        let awaitable = write_end.send_open_file_request(id, params)?;
        write_end.flush().await?;

        let (id, handle) = awaitable.wait().await?;

        Ok(File {
            sftp,
            write_end,
            handle,
            id: Some(id),
        })
    }
}

#[derive(Debug)]
pub struct File<'sftp, 's> {
    sftp: &'sftp Sftp<'s>,
    write_end: WriteEnd,
    handle: HandleOwned,
    id: Option<Id>,
}

impl File<'_, '_> {
    fn get_id_mut(&mut self) -> Id {
        self.id
            .take()
            .unwrap_or_else(|| self.sftp.get_thread_local_cached_id())
    }

    fn cache_id(&self, id: Id) {
        self.sftp.cache_id(id);
    }

    fn cache_id_mut(&mut self, id: Id) {
        if self.id.is_none() {
            self.id = Some(id);
        } else {
            self.cache_id(id);
        }
    }
}

impl Drop for File<'_, '_> {
    fn drop(&mut self) {
        let id = self.get_id_mut();
        let _ = self
            .write_end
            .send_close_request(id, Cow::Borrowed(&self.handle));
    }
}
