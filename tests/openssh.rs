use openssh::*;

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn it_works() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let stdout = session.command("echo foo").output().unwrap();
    assert_eq!(stdout.stdout, b"foo\n");
    assert!(stdout.stderr.is_empty());

    let stderr = session.command("echo foo > /dev/stderr").output().unwrap();
    assert!(stderr.stdout.is_empty());
    assert_eq!(stderr.stderr, b"foo\n");

    session.close().unwrap();
}
