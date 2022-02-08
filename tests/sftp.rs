mod common;
use common::*;

use openssh::sftp::*;

use std::cmp::{max, min};
use std::io::IoSlice;
use std::path::Path;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_io_utility::write_vectored_all;

use pretty_assertions::assert_eq;

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn sftp_init() {
    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test creating new file, truncating and opening existing file,
/// basic read, write and removal.
async fn sftp_file_basics() {
    let path = "/tmp/sftp_file_basics";
    let content = b"HELLO, WORLD!\n".repeat(200);

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();
        let content = &content[..min(sftp.max_write_len() as usize, content.len())];

        {
            let mut fs = sftp.fs("");

            // Create new file with Excl and write to it.
            debug_assert_eq!(
                sftp.options()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .await
                    .unwrap()
                    .write(&content)
                    .await
                    .unwrap(),
                content.len()
            );

            debug_assert_eq!(&*fs.read(path).await.unwrap(), &*content);

            // Create new file with Trunc and write to it.
            //
            // Sftp::Create opens the file truncated.
            debug_assert_eq!(
                sftp.create(path)
                    .await
                    .unwrap()
                    .write(&content)
                    .await
                    .unwrap(),
                content.len()
            );

            debug_assert_eq!(&*fs.read(path).await.unwrap(), &*content);

            // remove the file
            fs.remove_file(path).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

struct SftpFileWriteAllTester<'s> {
    path: &'static Path,
    file: File<'s>,
    fs: Fs<'s>,
    content: Vec<u8>,
}

impl<'s> SftpFileWriteAllTester<'s> {
    async fn new(sftp: &'s Sftp<'s>, path: &'static Path) -> SftpFileWriteAllTester<'s> {
        let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
        let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

        let file = sftp
            .options()
            .write(true)
            .read(true)
            .create(true)
            .open(path)
            .await
            .unwrap();
        let fs = sftp.fs("");

        Self {
            path,
            file,
            fs,
            content,
        }
    }

    async fn assert_content(mut self) {
        let len = self.content.len();

        self.file.rewind().await.unwrap();

        let buffer = self
            .file
            .read_all(len, BytesMut::with_capacity(len))
            .await
            .unwrap();

        assert_eq!(&*buffer, &*self.content);

        // remove the file
        self.fs.remove_file(self.path).await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all() {
    let path = Path::new("/tmp/sftp_file_write_all");

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        let mut tester = SftpFileWriteAllTester::new(&sftp, path).await;

        tester.file.write_all(&tester.content).await.unwrap();
        tester.assert_content().await;

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all_vectored() {
    let path = Path::new("/tmp/sftp_file_write_all_vectored");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        let mut tester = SftpFileWriteAllTester::new(&sftp, path).await;

        let content = &tester.content;
        let len = content.len();

        tester
            .file
            .write_all_vectorized(
                [
                    IoSlice::new(&content[..len / 2]),
                    IoSlice::new(&content[len / 2..]),
                ]
                .as_mut_slice(),
            )
            .await
            .unwrap();
        tester.assert_content().await;

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all_zero_copy() {
    let path = Path::new("/tmp/sftp_file_write_all_zero_copy");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        let mut tester = SftpFileWriteAllTester::new(&sftp, path).await;

        let content = &tester.content;
        let len = content.len();

        tester
            .file
            .write_all_zero_copy(
                [
                    BytesMut::from(&content[..len / 2]).freeze(),
                    BytesMut::from(&content[len / 2..]).freeze(),
                ]
                .as_mut_slice(),
            )
            .await
            .unwrap();
        tester.assert_content().await;

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test creating new TokioCompactFile, truncating and opening existing file,
/// basic read, write and removal.
async fn sftp_tokio_compact_file_basics() {
    let path = "/tmp/sftp_tokio_compact_file_basics";
    let content = b"HELLO, WORLD!\n".repeat(200);

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();
        let content = &content[..min(sftp.max_write_len() as usize, content.len())];

        let read_entire_file = || async {
            let mut buffer = Vec::with_capacity(content.len());

            let mut file: TokioCompactFile = sftp.open(path).await.unwrap().into();
            file.read_to_end(&mut buffer).await.unwrap();
            file.close().await.unwrap();

            buffer
        };

        {
            let mut fs = sftp.fs("");

            let mut file = sftp
                .options()
                .write(true)
                .create_new(true)
                .open(path)
                .await
                .map(TokioCompactFile::new)
                .unwrap();

            // Create new file with Excl and write to it.
            debug_assert_eq!(file.write(&content).await.unwrap(), content.len());

            file.flush().await.unwrap();
            file.close().await.unwrap();

            debug_assert_eq!(&*read_entire_file().await, &*content);

            // Create new file with Trunc and write to it.
            //
            // Sftp::Create opens the file truncated.
            let mut file = sftp.create(path).await.map(TokioCompactFile::new).unwrap();
            debug_assert_eq!(file.write(&content).await.unwrap(), content.len());

            // close also flush the internal future buffers, but using a
            // different implementation from `TokioCompactFile::poll_flush`
            // since it is executed in async context.
            //
            // Call `close` without calling `flush` first would force
            // `close` to do all the flush work, thus testing its implementation
            file.close().await.unwrap();

            debug_assert_eq!(&*read_entire_file().await, &*content);

            // remove the file
            fs.remove_file(path).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

struct SftpTokioCompactFileWriteAllTester<'s> {
    path: &'static Path,
    file: TokioCompactFile<'s>,
    fs: Fs<'s>,
    content: Vec<u8>,
}

impl<'s> SftpTokioCompactFileWriteAllTester<'s> {
    async fn new(
        sftp: &'s Sftp<'s>,
        path: &'static Path,
    ) -> SftpTokioCompactFileWriteAllTester<'s> {
        let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
        let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

        let file = sftp
            .options()
            .write(true)
            .read(true)
            .create(true)
            .open(path)
            .await
            .map(TokioCompactFile::new)
            .unwrap();
        let fs = sftp.fs("");

        Self {
            path,
            file,
            fs,
            content,
        }
    }

    async fn assert_content(mut self) {
        let len = self.content.len();

        self.file.rewind().await.unwrap();

        let buffer = self
            .file
            .read_all(len, BytesMut::with_capacity(len))
            .await
            .unwrap();

        assert_eq!(&*buffer, &*self.content);

        // remove the file
        self.fs.remove_file(self.path).await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_tokio_compact_file_write_all() {
    let path = Path::new("/tmp/sftp_tokio_compact_file_write_all");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        let mut tester = SftpTokioCompactFileWriteAllTester::new(&sftp, path).await;

        let content = &tester.content;

        tester.file.write_all(&content).await.unwrap();
        tester.assert_content().await;

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_tokio_compact_file_write_vectored_all() {
    let path = Path::new("/tmp/sftp_tokio_compact_file_write_vectored_all");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        let mut tester = SftpTokioCompactFileWriteAllTester::new(&sftp, path).await;

        let content = &tester.content;
        let len = content.len();

        write_vectored_all(
            &mut tester.file,
            [
                IoSlice::new(&content[..len / 2]),
                IoSlice::new(&content[len / 2..]),
            ]
            .as_mut_slice(),
        )
        .await
        .unwrap();
        tester.assert_content().await;

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::{set_len, set_permissions, metadata}.
async fn sftp_file_metadata() {
    let path = Path::new("/tmp/sftp_file_metadata");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        {
            let mut file = sftp
                .options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .await
                .unwrap();
            assert_eq!(file.metadata().await.unwrap().len().unwrap(), 0);

            file.set_len(28802).await.unwrap();
            assert_eq!(file.metadata().await.unwrap().len().unwrap(), 28802);
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::sync_all.
async fn sftp_file_sync_all() {
    let path = Path::new("/tmp/sftp_file_sync_all");

    for session in connects().await {
        let sftp = session
            .sftp(SftpOptions::new().max_write_len(200).max_read_len(200))
            .await
            .unwrap();

        sftp.create(path).await.unwrap().sync_all().await.unwrap();

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test creating, removing and iterating over dir, as well
/// as removing file.
async fn sftp_dir_basics() {
    let path = Path::new("/tmp/sftp_dir_basics");

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        {
            let mut fs = sftp.fs("");

            fs.create_dir(path).await.unwrap();

            fs.create_dir(path.join("dir")).await.unwrap();
            sftp.create(path.join("file")).await.unwrap();

            for entry in fs.open_dir(path).await.unwrap().read_dir().await.unwrap() {
                let filename = entry.filename().as_os_str();

                if filename == "." || filename == ".." {
                    continue;
                } else if filename == "dir" {
                    assert!(entry.file_type().unwrap().is_dir());
                } else if filename == "file" {
                    assert!(entry.file_type().unwrap().is_file());
                } else {
                    unreachable!("Unreachable!");
                }
            }

            fs.remove_file(path.join("file")).await.unwrap();
            fs.remove_dir(path.join("dir")).await.unwrap();
            fs.remove_dir(path).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test creation of symlink and canonicalize/read_link
async fn sftp_fs_symlink() {
    let filename = Path::new("/tmp/sftp_fs_symlink_file");
    let symlink = Path::new("/tmp/sftp_fs_symlink_symlink");

    let content = b"hello, world!\n";

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        {
            let mut fs = sftp.fs("");

            fs.write(filename, content).await.unwrap();
            fs.symlink(filename, symlink).await.unwrap();

            assert_eq!(&*fs.read(symlink).await.unwrap(), content);

            assert_eq!(fs.canonicalize(filename).await.unwrap(), filename);
            assert_eq!(fs.canonicalize(symlink).await.unwrap(), filename);
            assert_eq!(fs.read_link(symlink).await.unwrap(), filename);

            fs.remove_file(symlink).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test creation of hard_link and canonicalize
async fn sftp_fs_hardlink() {
    let filename = Path::new("/tmp/sftp_fs_hard_link_file");
    let hardlink = Path::new("/tmp/sftp_fs_hard_link_hardlink");

    let content = b"hello, world!\n";

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        {
            let mut fs = sftp.fs("");

            fs.write(filename, content).await.unwrap();
            fs.hard_link(filename, hardlink).await.unwrap();

            assert_eq!(&*fs.read(hardlink).await.unwrap(), content);

            assert_eq!(fs.canonicalize(filename).await.unwrap(), filename);
            assert_eq!(fs.canonicalize(hardlink).await.unwrap(), hardlink);

            fs.remove_file(hardlink).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}
