mod common;
use common::*;

use openssh::sftp::*;

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

    for session in connects().await {
        let sftp = session.sftp(SftpOptions::new()).await.unwrap();

        {
            let mut fs = sftp.fs(Some(""));

            let content = b"HELLO, WORLD!\n".repeat(200);

            // Create new file with Excl and write to it.
            sftp.options()
                .write(true)
                .create_new(true)
                .open(path)
                .await
                .unwrap()
                .write(&content)
                .await
                .unwrap();

            debug_assert_eq!(&*fs.read(path).await.unwrap(), &*content);

            // Create new file with Trunc and write to it.
            sftp.create(path)
                .await
                .unwrap()
                .write(&content)
                .await
                .unwrap();

            debug_assert_eq!(&*fs.read(path).await.unwrap(), &*content);

            // remove the file
            fs.remove_file(path).await.unwrap();
        }

        // close sftp and session
        sftp.close().await.unwrap();
        session.close().await.unwrap();
    }
}
