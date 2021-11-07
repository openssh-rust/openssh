use std::io;
use std::io::Write;
use std::str;

use assert_matches::assert_matches;
use once_cell::sync::OnceCell;

use regex::Regex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use openssh::*;

// TODO: how do we test the connection actually _failing_ so that the master reports an error?

fn addr() -> &'static str {
    static ADDR: OnceCell<Option<String>> = OnceCell::new();
    ADDR.get_or_init(|| std::env::var("TEST_HOST").ok())
        .as_deref()
        .unwrap_or("ssh://test-user@127.0.0.1:2222")
}

async fn session_builder_connect(builder: SessionBuilder) -> [Session; 2] {
    [
        builder.connect(&addr()).await.unwrap(),
        builder.connect_mux(&addr()).await.unwrap(),
    ]
}

async fn connects() -> [Session; 2] {
    [
        Session::connect(&addr(), KnownHosts::Accept).await.unwrap(),
        Session::connect_mux(&addr(), KnownHosts::Accept)
            .await
            .unwrap(),
    ]
}

async fn connects_err(host: &str) -> [Error; 2] {
    [
        Session::connect(host, KnownHosts::Accept)
            .await
            .unwrap_err(),
        Session::connect_mux(host, KnownHosts::Accept)
            .await
            .unwrap_err(),
    ]
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn it_connects() {
    for session in connects().await {
        session.check().await.unwrap();
        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn control_dir() {
    let dirname = std::path::Path::new("control-test");
    assert!(!dirname.exists());
    std::fs::create_dir(dirname).unwrap();

    let mut session_builder = SessionBuilder::default();
    session_builder.control_directory(&dirname);

    for session in session_builder_connect(session_builder).await {
        session.check().await.unwrap();
        let mut iter = std::fs::read_dir(&dirname).unwrap();
        assert!(iter.next().is_some());
        session.close().await.unwrap();
    }

    std::fs::remove_dir(&dirname).unwrap();
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ProtoUserHostPort<'a> {
    proto: Option<&'a str>,
    user: Option<&'a str>,
    host: Option<&'a str>,
    port: Option<&'a str>,
}

fn parse_user_host_port<'a>(s: &'a str) -> Option<ProtoUserHostPort> {
    static SSH_REGEX: OnceCell<Regex> = OnceCell::new();

    let ssh_regex = SSH_REGEX.get_or_init(|| {
        Regex::new(
            r#"(?x)^((?P<proto>[[:alpha:]]+)://)?((?P<user>.*?)@)?(?P<host>.*?)(:(?P<port>\d+))?$"#,
        )
        .unwrap()
    });

    ssh_regex.captures(s).and_then(|cap| {
        Some(ProtoUserHostPort {
            proto: cap.name("proto").and_then(|m| Some(m.as_str())),
            user: cap.name("user").and_then(|m| Some(m.as_str())),
            host: cap.name("host").and_then(|m| Some(m.as_str())),
            port: cap.name("port").and_then(|m| Some(m.as_str())),
        })
    })
}

#[test]
fn test_parse_proto_user_host_port() {
    let addr = "ssh://test-user@127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    assert_eq!("ssh", parsed_addr.proto.unwrap());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_user_host_port() {
    let addr = "test-user@127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_user_host() {
    let addr = "test-user@127.0.0.1";
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert!(parsed_addr.port.is_none());
}

#[test]
fn test_parse_host_port() {
    let addr = "127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert!(parsed_addr.user.is_none());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_host() {
    let addr = "127.0.0.1";
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert!(parsed_addr.user.is_none());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert!(parsed_addr.port.is_none());
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn config_file() {
    let dirname = std::path::Path::new("config-file-test");
    let ssh_config_file = dirname.join("alternate_ssh_config");
    assert!(!dirname.exists());
    assert!(!ssh_config_file.exists());
    std::fs::create_dir(dirname).unwrap();

    let addr = addr();
    let parsed_addr = parse_user_host_port(&addr).unwrap();
    let ssh_config_contents = format!(
        r#"Host config-file-test
        User {}
        HostName {}
        Port {}"#,
        parsed_addr.user.unwrap_or("test-user"),
        parsed_addr.host.unwrap_or("127.0.0.1"),
        parsed_addr.port.unwrap_or("2222")
    );
    let mut ssh_config_handle = std::fs::File::create(&ssh_config_file).unwrap();
    ssh_config_handle
        .write_all(ssh_config_contents.as_bytes())
        .unwrap();

    let mut session_builder = SessionBuilder::default();

    session_builder
        .known_hosts_check(KnownHosts::Accept)
        .config_file(&ssh_config_file);

    // this host name is resolved by the custom ssh_config.
    let sessions = [
        session_builder.connect("config-file-test").await.unwrap(),
        session_builder
            .connect_mux("config-file-test")
            .await
            .unwrap(),
    ];

    for session in sessions {
        session.check().await.unwrap();
        session.close().await.unwrap();
    }

    std::fs::remove_dir_all(&dirname).unwrap();
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn terminate_on_drop() {
    drop(Session::connect(&addr(), KnownHosts::Add).await.unwrap());
    drop(
        Session::connect_mux(&addr(), KnownHosts::Add)
            .await
            .unwrap(),
    );
    // NOTE: how do we test that it actually killed the master here?
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn stdout() {
    for session in connects().await {
        let child = session.command("echo").arg("foo").output().await.unwrap();
        assert_eq!(child.stdout, b"foo\n");

        let child = session
            .command("echo")
            .arg("foo")
            .raw_arg(">")
            .arg("/dev/stderr")
            .output()
            .await
            .unwrap();
        assert!(child.stdout.is_empty());

        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn stderr() {
    for session in connects().await {
        let child = session.command("echo").arg("foo").output().await.unwrap();
        assert!(child.stderr.is_empty());

        let child = session
            .command("echo")
            .arg("foo")
            .raw_arg(">")
            .arg("/dev/stderr")
            .output()
            .await
            .unwrap();
        assert_eq!(child.stderr, b"foo\n");

        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn stdin() {
    for session in connects().await {
        let mut child = session
            .command("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .await
            .unwrap();

        // write something to standard in and send EOF
        let mut stdin = child.stdin().take().unwrap();
        stdin.write_all(b"hello world").await.unwrap();
        drop(stdin);

        // cat should print it back on stdout
        let mut stdout = child.stdout().take().unwrap();
        let mut out = String::new();
        stdout.read_to_string(&mut out).await.unwrap();
        assert_eq!(out, "hello world");
        drop(stdout);

        // cat should now have terminated
        let status = child.wait().await.unwrap();
        drop(child);

        // ... successfully
        assert!(status.success());

        session.close().await.unwrap();
    }
}

macro_rules! assert_kind {
    ($e:expr, $kind:expr) => {
        let e = $e;
        let kind = $kind;

        assert_matches!(
            e,
            Error::Remote(ref e) if e.kind() == kind,
            "{:?}",
            e
        );
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn sftp_can() {
    for session in connects().await {
        let mut sftp = session.sftp();

        // first, do some access checks
        // some things we can do
        sftp.can(Mode::Write, "test_file").await.unwrap();
        sftp.can(Mode::Write, ".ssh/test_file").await.unwrap();
        sftp.can(Mode::Read, ".ssh/authorized_keys").await.unwrap();
        sftp.can(Mode::Read, "/etc/hostname").await.unwrap();
        // some things we cannot
        assert_kind!(
            sftp.can(Mode::Write, "/etc/passwd").await.unwrap_err(),
            io::ErrorKind::PermissionDenied
        );
        assert_kind!(
            sftp.can(Mode::Write, "no/such/file").await.unwrap_err(),
            io::ErrorKind::NotFound
        );
        assert_kind!(
            sftp.can(Mode::Read, "/etc/shadow").await.unwrap_err(),
            io::ErrorKind::PermissionDenied
        );
        assert_kind!(
            sftp.can(Mode::Read, "/etc/no-such-file").await.unwrap_err(),
            io::ErrorKind::NotFound
        );
        assert_kind!(
            sftp.can(Mode::Write, "/etc/no-such-file")
                .await
                .unwrap_err(),
            io::ErrorKind::PermissionDenied
        );
        assert_kind!(
            sftp.can(Mode::Write, "/no-such-file").await.unwrap_err(),
            io::ErrorKind::PermissionDenied
        );
        assert_kind!(
            sftp.can(Mode::Read, "no/such/file").await.unwrap_err(),
            io::ErrorKind::NotFound
        );
        // and something are just weird
        assert_kind!(
            sftp.can(Mode::Write, ".ssh").await.unwrap_err(),
            io::ErrorKind::AlreadyExists
        );
        assert_kind!(
            sftp.can(Mode::Write, "/etc").await.unwrap_err(),
            io::ErrorKind::AlreadyExists
        );
        assert_kind!(
            sftp.can(Mode::Write, "/").await.unwrap_err(),
            io::ErrorKind::AlreadyExists
        );
        assert_kind!(
            sftp.can(Mode::Read, "/etc").await.unwrap_err(),
            io::ErrorKind::Other
        );

        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn sftp() {
    for session in connects().await {
        let mut sftp = session.sftp();

        // first, open a file for writing
        let mut w = sftp.write_to("test_file").await.unwrap();

        // reading from a write-only file should error
        let failed = w.read(&mut [0]).await.unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::UnexpectedEof);

        // write something to the file
        w.write_all(b"hello").await.unwrap();
        w.close().await.unwrap();

        // we should still be able to write it
        sftp.can(Mode::Write, "test_file").await.unwrap();
        // and now also read it
        sftp.can(Mode::Read, "test_file").await.unwrap();

        // open the file again for appending
        let mut w = sftp.append_to("test_file").await.unwrap();

        // reading from an append-only file should also error
        let failed = w.read(&mut [0]).await.unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::UnexpectedEof);

        // append something to the file
        w.write_all(b" world").await.unwrap();
        w.close().await.unwrap();

        // then, open the same file for reading
        let mut r = sftp.read_from("test_file").await.unwrap();

        // writing to a read-only file should error
        let failed = r.write(&[0]).await.unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::WriteZero);

        // read back the file
        let mut contents = String::new();
        r.read_to_string(&mut contents).await.unwrap();
        assert_eq!(contents, "hello world");
        r.close().await.unwrap();

        // reading a file that does not exist should error on open
        let failed = sftp.read_from("no/such/file").await.unwrap_err();
        assert_kind!(failed, io::ErrorKind::NotFound);
        // so should file we're not allowed to read
        let failed = sftp.read_from("/etc/shadow").await.unwrap_err();
        assert_kind!(failed, io::ErrorKind::PermissionDenied);

        // writing a file that does not exist should also error on open
        let failed = sftp.write_to("no/such/file").await.unwrap_err();
        assert_kind!(failed, io::ErrorKind::NotFound);
        // so should file we're not allowed to write
        let failed = sftp.write_to("/rootfile").await.unwrap_err();
        assert_kind!(failed, io::ErrorKind::PermissionDenied);

        // writing to a full disk (or the like) should also error
        let mut w = sftp.write_to("/dev/full").await.unwrap();
        w.write_all(b"hello world").await.unwrap();
        let failed = w.close().await.unwrap_err();
        assert_kind!(failed, io::ErrorKind::WriteZero);

        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn bad_remote_command() {
    for session in connects().await {
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
        let failed = child.try_wait().await.unwrap_err();
        eprintln!("{:?}", failed);
        assert!(matches!(failed, Error::Remote(ref e) if e.kind() == io::ErrorKind::NotFound));
        child.disconnect().await.unwrap_err();

        session.close().await.unwrap();
    }
}

#[tokio::test]
async fn connect_timeout() {
    use std::time::{Duration, Instant};

    let mut sb = SessionBuilder::default();
    let sb = sb.connect_timeout(Duration::from_secs(1));

    let host = "192.0.0.8";

    // Test process_impl
    let t = Instant::now();
    let failed = sb.connect(host).await.unwrap_err();
    assert!(t.elapsed() > Duration::from_secs(1));
    assert!(t.elapsed() < Duration::from_secs(2));
    eprintln!("{:?}", failed);
    assert_matches!(failed, Error::Connect(e) if e.kind() == io::ErrorKind::TimedOut);

    // Test mux_client_impl
    let t = Instant::now();
    let failed = sb.connect_mux(host).await.unwrap_err();
    assert!(t.elapsed() > Duration::from_secs(1));
    assert!(t.elapsed() < Duration::from_secs(2));
    eprintln!("{:?}", failed);
    assert_matches!(failed, Error::Connect(e) if e.kind() == io::ErrorKind::TimedOut);
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn spawn_and_wait() {
    use std::time::{Duration, Instant};

    for session in connects().await {
        let t = Instant::now();
        let sleeping1 = session.command("sleep").arg("1").spawn().await.unwrap();
        let sleeping2 = sleeping1
            .session()
            .command("sleep")
            .arg("2")
            .spawn()
            .await
            .unwrap();
        sleeping1.wait_with_output().await.unwrap();
        assert!(t.elapsed() > Duration::from_secs(1));
        sleeping2.wait_with_output().await.unwrap();
        assert!(t.elapsed() > Duration::from_secs(2));

        session.close().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn escaping() {
    for session in connects().await {
        let status = dbg!(session
            .command("printf")
            .arg("%d %d")
            .arg("1")
            .arg("2")
            .output()
            .await
            .unwrap())
        .status;
        assert!(status.success());

        let status = dbg!(session
            .command("printf")
            .args(vec!["%d %d", "1", "2"])
            .output()
            .await
            .unwrap())
        .status;
        assert!(status.success());

        let status = dbg!(session
            .command("printf")
            .arg("%d %d")
            .raw_arg("1 2")
            .output()
            .await
            .unwrap())
        .status;
        assert!(status.success());

        let status = dbg!(session
            .command("printf")
            .arg("%d %d")
            .raw_args(std::iter::once("1 2"))
            .output()
            .await
            .unwrap())
        .status;
        assert!(status.success());

        let status = dbg!(session
            .raw_command("printf '%d %d'")
            .arg("1")
            .arg("2")
            .output()
            .await
            .unwrap())
        .status;
        assert!(status.success());

        session.close().await.unwrap();
    }
}

#[tokio::test]
async fn cannot_resolve() {
    for err in connects_err("bat-host").await {
        match err {
            Error::Connect(e) => {
                eprintln!("{:?}", e);
                assert_eq!(e.kind(), io::ErrorKind::Other);
            }
            e => unreachable!("{:?}", e),
        }
    }
}

#[tokio::test]
async fn no_route() {
    for err in connects_err("255.255.255.255").await {
        match err {
            Error::Connect(e) => {
                eprintln!("{:?}", e);
                assert_eq!(e.kind(), io::ErrorKind::Other);
            }
            e => unreachable!("{:?}", e),
        }
    }
}

#[tokio::test]
async fn connection_refused() {
    for err in connects_err("ssh://127.0.0.1:9").await {
        match err {
            Error::Connect(e) => {
                eprintln!("{:?}", e);
                assert_eq!(e.kind(), io::ErrorKind::ConnectionRefused);
            }
            e => unreachable!("{:?}", e),
        }
    }
}

#[tokio::test]
async fn auth_failed() {
    let addr = if cfg!(ci) {
        // prefer the known-accessible test server when available
        addr().replace("test-user", "bad-user")
    } else {
        String::from("ssh://openssh-tester@login.csail.mit.edu")
    };

    for err in connects_err(&addr).await {
        match err {
            Error::Connect(e) => {
                eprintln!("{:?}", e);
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
            }
            e => unreachable!("{:?}", e),
        }
    }
}
