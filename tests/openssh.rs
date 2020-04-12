use openssh::*;
use std::io;

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

#[test]
fn cannot_resolve() {
    match Session::connect("bad-host", KnownHosts::Accept).unwrap_err() {
        Error::Connect(e) => {
            eprintln!("{:?}", e);
            assert_eq!(e.kind(), io::ErrorKind::Other);
        }
        e => unreachable!("{:?}", e),
    }
}

#[test]
fn no_route() {
    match Session::connect("255.255.255.255", KnownHosts::Accept).unwrap_err() {
        Error::Connect(e) => {
            eprintln!("{:?}", e);
            assert_eq!(e.kind(), io::ErrorKind::Other);
        }
        e => unreachable!("{:?}", e),
    }
}

#[test]
fn connection_refused() {
    match Session::connect("ssh://127.0.0.1:9", KnownHosts::Accept).unwrap_err() {
        Error::Connect(e) => {
            eprintln!("{:?}", e);
            assert_eq!(e.kind(), io::ErrorKind::ConnectionRefused);
        }
        e => unreachable!("{:?}", e),
    }
}

#[test]
fn auth_failed() {
    let addr = if cfg!(ci) {
        // prefer the known-accessible test server when available
        addr().replace("test-user", "bad-user")
    } else {
        String::from("ssh://openssh-tester@login.csail.mit.edu")
    };

    match Session::connect(&addr, KnownHosts::Accept).unwrap_err() {
        Error::Connect(e) => {
            eprintln!("{:?}", e);
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        }
        e => unreachable!("{:?}", e),
    }
}
