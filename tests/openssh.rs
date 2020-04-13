use openssh::*;
use std::io::{self, prelude::*};
use std::process::Stdio;

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or("ssh://test-user@127.0.0.1:2222".to_string())
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn it_connects() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();
    session.check().unwrap();
    session.close().unwrap();
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn stdout() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let child = session.command("echo").arg("foo").output().unwrap();
    assert_eq!(child.stdout, b"foo\n");

    let child = session
        .command("echo")
        .arg("foo")
        .arg(">")
        .arg("/dev/stderr")
        .output()
        .unwrap();
    assert!(child.stdout.is_empty());

    session.close().unwrap();
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn stderr() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let child = session.command("echo").arg("foo").output().unwrap();
    assert!(child.stderr.is_empty());

    let child = session
        .command("echo")
        .arg("foo")
        .arg(">")
        .arg("/dev/stderr")
        .output()
        .unwrap();
    assert_eq!(child.stderr, b"foo\n");

    session.close().unwrap();
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn stdin() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let mut child = session
        .command("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    // write something to standard in and send EOF
    let mut stdin = child.stdin().take().unwrap();
    write!(stdin, "hello world").unwrap();
    drop(stdin);

    // cat should print it back on stdout
    let mut stdout = child.stdout().take().unwrap();
    let mut out = String::new();
    stdout.read_to_string(&mut out).unwrap();
    assert_eq!(out, "hello world");
    drop(stdout);

    // cat should now have terminated
    let status = child.wait().unwrap();
    drop(child);

    // ... successfully
    assert!(status.success());

    session.close().unwrap();
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn bad_remote_command() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    // a bad remote command should result in a _local_ error.
    let failed = session.command("no such program").output().unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    eprintln!("{:?}", failed);

    // no matter how you run it
    let failed = session.command("no such program").status().unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    eprintln!("{:?}", failed);

    session.close().unwrap();
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn broken_connection() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let sleeping = session.command("sleep").arg("1000").spawn().unwrap();

    // get ID of remote ssh process
    let ppid = session.command("echo").arg("$PPID").output().unwrap();
    eprintln!("ppid: {:?}", ppid);
    let ppid: u32 = String::from_utf8(ppid.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // and kill it -- this kills the master connection
    let killed = session
        .command("kill")
        .arg("-9")
        .arg(&format!("{}", ppid))
        .output()
        .unwrap_err();
    assert!(matches!(killed, Error::Disconnected));

    // this fails because the master connection is gone
    let failed = session.command("echo").arg("foo").output().unwrap_err();
    assert!(matches!(failed, Error::Disconnected));

    // so does this
    let failed = session.command("echo").arg("foo").status().unwrap_err();
    assert!(matches!(failed, Error::Disconnected));

    // the spawned child we're waiting for must also have failed
    let failed = sleeping.wait_with_output().unwrap_err();
    assert!(matches!(failed, Error::Disconnected));

    // check should obviously fail
    let failed = session.check().unwrap_err();
    assert!(matches!(failed, Error::Disconnected));

    // what should close do in this instance?
    // probably not return an error, since the connection _is_ closed.
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
