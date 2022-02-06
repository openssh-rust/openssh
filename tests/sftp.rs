mod common;
use common::*;

use openssh::sftp::*;

use std::cmp::{max, min};
use std::path::Path;

use bytes::BytesMut;
use tokio::io::AsyncSeekExt;

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
            let mut fs = sftp.fs(Some(""));

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

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
/// Test File::write_all, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all() {
    let path = "/tmp/sftp_file_write_all";

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
        let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

        {
            let mut file = sftp
                .options()
                .write(true)
                .read(true)
                .create(true)
                .open(path)
                .await
                .unwrap();
            let mut fs = sftp.fs(Some(""));

            file.write_all(&content).await.unwrap();
            file.rewind().await.unwrap();

            let buffer = file
                .read_all(content.len(), BytesMut::with_capacity(content.len()))
                .await
                .unwrap();

            assert_eq!(&*buffer, &*content);

            // remove the file
            fs.remove_file(path).await.unwrap();
        }

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
            let mut fs = sftp.fs(Some(""));

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
