use openssh::*;

// TODO: how do we test the connection actually _failing_ so that the master reports an error?

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
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
