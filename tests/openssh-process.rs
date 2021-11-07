use std::io;

use openssh::*;

// TODO: how do we test the connection actually _failing_ so that the master reports an error?

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn shell() {
    let session = Session::connect(&addr(), KnownHosts::Accept).await.unwrap();

    let child = session.shell("echo $USER").output().await.unwrap();
    assert_eq!(child.stdout, b"test-user\n");

    let child = session
        .shell(r#"touch "$USER Documents""#)
        .status()
        .await
        .unwrap();
    assert!(child.success());

    let child = session
        .shell(r#"rm test-user\ Documents"#)
        .status()
        .await
        .unwrap();
    assert!(child.success());

    let child = session.shell("echo \\$SHELL").output().await.unwrap();
    assert_eq!(child.stdout, b"$SHELL\n");

    let child = session
        .shell(r#"echo $USER | grep -c test"#)
        .status()
        .await
        .unwrap();
    assert!(child.success());

    session.close().await.unwrap();
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn bad_remote_command() {
    let session = Session::connect(&addr(), KnownHosts::Accept).await.unwrap();

    // a bad remote command should result in a _local_ error.
    let failed = session
        .command("no such program")
        .output()
        .await
        .unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));

    // no matter how you run it
    let failed = session
        .command("no such program")
        .status()
        .await
        .unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));

    // even if you spawn first
    let mut child = session.command("no such program").spawn().await.unwrap();
    let failed = child.wait().await.unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    child.disconnect().await.unwrap_err();

    // of if you want output
    let child = session.command("no such program").spawn().await.unwrap();
    let failed = child.wait_with_output().await.unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));

    // no matter how hard you _try_
    let mut child = session.command("no such program").spawn().await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let failed = child.try_wait().unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    child.disconnect().await.unwrap_err();

    session.close().await.unwrap();
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn process_exit_on_signal() {
    let session = Session::connect(&addr(), KnownHosts::Accept).await.unwrap();

    let mut sleeping = session.command("sleep").arg("5566").spawn().await.unwrap();

    // give it some time to make sure it starts
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Now stop that process.
    //
    // We use `pkill -f` to match on the number rather than the `sleep` command, since other tests
    // may use `sleep`. We use `-o` to ensure that we don't accidentally kill the ssh connection
    // itself, but instead match the _oldest_ matching command.
    let killed = session
        .command("pkill")
        .arg("-f")
        .arg("-o")
        .arg("5566")
        .output()
        .await
        .unwrap();
    eprintln!("{:?}", killed);
    assert!(killed.status.success());

    // await that process — this will yield "Disconnected", since the remote process disappeared
    let failed = sleeping.wait().await.unwrap_err();
    eprintln!("{:?}", failed);
    assert!(matches!(failed, Error::Disconnected));

    // the connection should still work though
    let _ = session.check().await.unwrap();
}
