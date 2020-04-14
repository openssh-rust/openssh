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

macro_rules! assert_kind {
    ($e:expr, $kind:expr) => {
        let e = $e;
        assert!(
            matches!(e, Error::Remote(ref e) if e.kind() == $kind),
            "{:?}",
            e
        );
    }
}

#[test]
#[cfg_attr(not(ci), ignore)]
fn sftp() {
    let session = Session::connect(&addr(), KnownHosts::Accept).unwrap();

    let mut sftp = session.sftp();

    // first, do some access checks
    // some things we can do
    sftp.can(Mode::Write, "test_file").unwrap();
    sftp.can(Mode::Read, "/etc/hostname").unwrap();
    // some things we cannot
    assert_kind!(
        sftp.can(Mode::Write, "/etc/passwd").unwrap_err(),
        io::ErrorKind::PermissionDenied
    );
    assert_kind!(
        sftp.can(Mode::Write, "no/such/file").unwrap_err(),
        io::ErrorKind::NotFound
    );
    assert_kind!(
        sftp.can(Mode::Read, "/etc/shadow").unwrap_err(),
        io::ErrorKind::PermissionDenied
    );
    assert_kind!(
        sftp.can(Mode::Read, "no/such/file").unwrap_err(),
        io::ErrorKind::NotFound
    );
    // and something are just weird
    assert_kind!(
        sftp.can(Mode::Write, ".ssh").unwrap_err(),
        io::ErrorKind::AlreadyExists
    );
    assert_kind!(
        sftp.can(Mode::Write, "/etc").unwrap_err(),
        io::ErrorKind::AlreadyExists
    );
    assert_kind!(
        sftp.can(Mode::Write, "/").unwrap_err(),
        io::ErrorKind::AlreadyExists
    );
    assert_kind!(
        sftp.can(Mode::Read, "/etc").unwrap_err(),
        io::ErrorKind::Other
    );

    // first, open a file for writing
    let mut w = sftp.write_to("test_file").unwrap();

    // reading from a write-only file should error
    let failed = w.read(&mut [0]).unwrap_err();
    assert_eq!(failed.kind(), io::ErrorKind::UnexpectedEof);

    // write something to the file
    write!(w, "hello world").unwrap();
    w.close().unwrap();

    // we should still be able to write it
    sftp.can(Mode::Write, "test_file").unwrap();
    // and now also read it
    sftp.can(Mode::Read, "test_file").unwrap();

    // then, open the same file for reading
    let mut r = sftp.read_from("test_file").unwrap();

    // writing to a read-only file should error
    let failed = r.write(&[0]).unwrap_err();
    assert_eq!(failed.kind(), io::ErrorKind::WriteZero);

    // read back the file
    let mut contents = String::new();
    r.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "hello world");
    r.close().unwrap();

    // reading a file that does not exist should error on open
    let failed = sftp.read_from("no/such/file").unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    // so should file we're not allowed to read
    let failed = sftp.read_from("/etc/shadow").unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::PermissionDenied));

    // writing a file that does not exist should also error on open
    let failed = sftp.write_to("no/such/file").unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
    // so should file we're not allowed to write
    let failed = sftp.write_to("/rootfile").unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::PermissionDenied));

    // writing to a full disk (or the like) should also error
    let mut w = sftp.write_to("/dev/full").unwrap();
    w.write_all(b"hello world").unwrap();
    let failed = w.close().unwrap_err();
    assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::WriteZero));

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
