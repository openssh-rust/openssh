use super::{Buffer, Error, Id, Sftp, WriteEnd};

use std::borrow::Cow;
use std::convert::TryInto;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncSeek;

use openssh_sftp_client::{
    AwaitableDataFuture, AwaitableStatusFuture, CreateFlags, FileAttrs, HandleOwned,
};

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

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
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

        let (id, handle) = write_end.send_open_file_request(id, params)?.wait().await?;

        Ok(File {
            sftp,
            write_end,
            handle,
            id: Some(id),

            is_readable: self.options.get_read(),
            is_writable: self.options.get_write(),

            buffer: Vec::new(),
            offset: 0,
            future: FileFuture::None,
        })
    }
}

#[derive(Debug)]
enum FileFuture<Buffer: Send + Sync> {
    None,
    Data(AwaitableDataFuture<Buffer>),
    Status(AwaitableStatusFuture<Buffer>),
}

#[derive(Debug)]
pub struct File<'sftp, 's> {
    sftp: &'sftp Sftp<'s>,
    write_end: WriteEnd,
    handle: HandleOwned,
    id: Option<Id>,

    is_readable: bool,
    is_writable: bool,

    buffer: Vec<u8>,
    offset: u64,
    future: FileFuture<Buffer>,
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

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_len(&mut self, size: u64) -> Result<(), Error> {
        let id = self.get_id_mut();

        let mut attrs = FileAttrs::new();
        attrs.set_size(size);

        let id = self
            .write_end
            .send_fsetstat_request(id, Cow::Borrowed(&self.handle), attrs)?
            .wait()
            .await?
            .0;

        self.cache_id_mut(id);

        Ok(())
    }
}

impl AsyncSeek for File<'_, '_> {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        use io::SeekFrom::*;

        match position {
            Start(pos) => {
                if pos == self.offset {
                    return Ok(());
                }

                self.offset = pos;
            }
            End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Seeking from the end is unsupported",
                ));
            }
            Current(n) => {
                if n == 0 {
                    return Ok(());
                } else if n > 0 {
                    self.offset =
                        self.offset
                            .checked_add(n.try_into().unwrap())
                            .ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Overflow occured during seeking",
                                )
                            })?;
                } else {
                    self.offset = self
                        .offset
                        .checked_sub((-n).try_into().unwrap())
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Underflow occured during seeking",
                            )
                        })?;
                }
            }
        }

        // Reset future since they are invalidated by change of offset.
        self.future = FileFuture::None;

        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.offset))
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
