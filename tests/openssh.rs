use lazy_static::lazy_static;
use regex::Regex;
use std::borrow::Cow;
use std::env;
use std::io;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{
    net::{UnixListener, UnixStream},
    time::sleep,
};

use openssh::*;

// TODO: how do we test the connection actually _failing_ so that the master reports an error?

fn addr() -> String {
    std::env::var("TEST_HOST").unwrap_or_else(|_| "ssh://test-user@127.0.0.1:2222".to_string())
}

fn loopback() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

fn get_known_hosts_path() -> PathBuf {
    let mut path = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    path.push("openssh-rs/known_hosts");
    path
}

async fn session_builder_connect(mut builder: SessionBuilder, addr: &str) -> Vec<Session> {
    let mut sessions = Vec::with_capacity(2);

    builder.user_known_hosts_file(get_known_hosts_path());

    #[cfg(feature = "process-mux")]
    {
        sessions.push(builder.connect(addr).await.unwrap());
    }

    #[cfg(feature = "native-mux")]
    {
        sessions.push(builder.connect_mux(addr).await.unwrap());
    }

    sessions
}

async fn connects() -> Vec<Session> {
    let mut sessions = Vec::with_capacity(2);

    let mut builder = SessionBuilder::default();

    builder
        .user_known_hosts_file(get_known_hosts_path())
        .known_hosts_check(KnownHosts::Accept);

    #[cfg(feature = "process-mux")]
    {
        sessions.push(builder.connect(&addr()).await.unwrap());
    }

    #[cfg(feature = "native-mux")]
    {
        sessions.push(builder.connect_mux(&addr()).await.unwrap());
    }

    sessions
}

async fn connects_err(host: &str) -> Vec<Error> {
    let mut builder = SessionBuilder::default();

    builder
        .user_known_hosts_file(get_known_hosts_path())
        .known_hosts_check(KnownHosts::Accept);

    let mut errors = Vec::with_capacity(2);

    #[cfg(feature = "process-mux")]
    {
        errors.push(builder.connect(host).await.unwrap_err());
    }

    #[cfg(feature = "native-mux")]
    {
        errors.push(builder.connect_mux(host).await.unwrap_err());
    }

    errors
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

    for session in session_builder_connect(session_builder, &addr()).await {
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

fn parse_user_host_port(s: &str) -> Option<ProtoUserHostPort> {
    lazy_static! {
        static ref SSH_REGEX: Regex = Regex::new(
            r"(?x)^((?P<proto>[[:alpha:]]+)://)?((?P<user>.*?)@)?(?P<host>.*?)(:(?P<port>\d+))?$"
        )
        .unwrap();
    }

    SSH_REGEX.captures(s).map(|cap| ProtoUserHostPort {
        proto: cap.name("proto").map(|m| m.as_str()),
        user: cap.name("user").map(|m| m.as_str()),
        host: cap.name("host").map(|m| m.as_str()),
        port: cap.name("port").map(|m| m.as_str()),
    })
}

#[test]
fn test_parse_proto_user_host_port() {
    let addr = "ssh://test-user@127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(addr).unwrap();
    assert_eq!("ssh", parsed_addr.proto.unwrap());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_user_host_port() {
    let addr = "test-user@127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_user_host() {
    let addr = "test-user@127.0.0.1";
    let parsed_addr = parse_user_host_port(addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert_eq!("test-user", parsed_addr.user.unwrap());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert!(parsed_addr.port.is_none());
}

#[test]
fn test_parse_host_port() {
    let addr = "127.0.0.1:2222";
    let parsed_addr = parse_user_host_port(addr).unwrap();
    assert!(parsed_addr.proto.is_none());
    assert!(parsed_addr.user.is_none());
    assert_eq!("127.0.0.1", parsed_addr.host.unwrap());
    assert_eq!("2222", parsed_addr.port.unwrap());
}

#[test]
fn test_parse_host() {
    let addr = "127.0.0.1";
    let parsed_addr = parse_user_host_port(addr).unwrap();
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
    for session in session_builder_connect(session_builder, "config-file-test").await {
        session.check().await.unwrap();
        session.close().await.unwrap();
    }

    std::fs::remove_dir_all(&dirname).unwrap();
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn terminate_on_drop() {
    let mut builder = SessionBuilder::default();

    builder
        .user_known_hosts_file(get_known_hosts_path())
        .known_hosts_check(KnownHosts::Add);

    #[cfg(feature = "process-mux")]
    {
        drop(builder.connect(&addr()).await.unwrap());
    }

    #[cfg(feature = "native-mux")]
    {
        drop(builder.connect_mux(&addr()).await.unwrap());
    }
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
async fn shell() {
    for session in connects().await {
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
            .output()
            .await
            .unwrap();
        eprintln!("shell: {:#?}", child);
        assert!(child.status.success());

        let child = session.shell("echo \\$SHELL").output().await.unwrap();
        assert_eq!(child.stdout, b"$SHELL\n");

        let child = session
            .shell(r#"echo $USER | grep -c test"#)
            .status()
            .await
            .unwrap();
        eprintln!("shell: {:#?}", child);
        assert!(child.success());

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

        // ... successfully
        assert!(status.success());

        session.close().await.unwrap();
    }
}

macro_rules! assert_remote_kind {
    ($e:expr, $kind:expr) => {
        let e = $e;

        assert!(
            matches!(e, Error::Remote(ref e) if e.kind() == $kind),
            "{:?}",
            e
        );
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
        assert_remote_kind!(failed, io::ErrorKind::NotFound);

        // no matter how you run it
        let failed = session
            .command("no such program")
            .status()
            .await
            .unwrap_err();
        eprintln!("{:?}", failed);
        assert_remote_kind!(failed, io::ErrorKind::NotFound);

        // even if you spawn first
        let child = session.command("no such program").spawn().await.unwrap();
        let failed = child.wait().await.unwrap_err();
        eprintln!("{:?}", failed);
        assert_remote_kind!(failed, io::ErrorKind::NotFound);

        // of if you want output
        let child = session.command("no such program").spawn().await.unwrap();
        let failed = child.wait_with_output().await.unwrap_err();
        eprintln!("{:?}", failed);
        assert_remote_kind!(failed, io::ErrorKind::NotFound);

        session.close().await.unwrap();
    }
}

#[tokio::test]
async fn connect_timeout() {
    use std::time::{Duration, Instant};

    let mut sb = SessionBuilder::default();
    sb.connect_timeout(Duration::from_secs(1))
        .user_known_hosts_file(get_known_hosts_path());

    let host = "192.0.0.8";

    // Test process_impl
    #[cfg(feature = "process-mux")]
    {
        let t = Instant::now();
        let res = sb.connect(host).await;
        let duration = t.elapsed();

        let failed = res.unwrap_err();

        assert!(duration > Duration::from_secs(1));
        assert!(duration < Duration::from_secs(2));
        eprintln!("{:?}", failed);
        assert!(matches!(failed, Error::Connect(ref e) if e.kind() == io::ErrorKind::TimedOut));
    }

    // Test native-mux_impl
    #[cfg(feature = "native-mux")]
    {
        let t = Instant::now();
        let res = sb.connect_mux(host).await;
        let duration = t.elapsed();

        let failed = res.unwrap_err();

        assert!(duration > Duration::from_secs(1));
        assert!(duration < Duration::from_secs(2));
        eprintln!("{:?}", failed);
        assert!(matches!(failed, Error::Connect(ref e) if e.kind() == io::ErrorKind::TimedOut));
    }
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
#[cfg_attr(not(ci), ignore)]
async fn process_exit_on_signal() {
    for session in connects().await {
        let sleeping = session.command("sleep").arg("5566").spawn().await.unwrap();

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
        eprintln!("process_exit_on_signal: {:?}", killed);
        assert!(killed.status.success());

        // await that process — this will yield "RemoteProcessTerminated", since the remote process disappeared
        let failed = sleeping.wait().await.unwrap_err();
        eprintln!("{:?}", failed);
        assert!(matches!(failed, Error::RemoteProcessTerminated));

        // the connection should still work though
        let _ = session.check().await.unwrap();
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn broken_connection() {
    for session in connects().await {
        let sleeping = session.command("sleep").arg("1000").spawn().await.unwrap();

        // get ID of remote ssh process
        let ppid = session
            .command("echo")
            .raw_arg("$PPID")
            .output()
            .await
            .unwrap();
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
            .await
            .unwrap_err();
        eprintln!("{:?}", killed);
        assert!(matches!(killed, Error::RemoteProcessTerminated));

        // this fails because the master connection is gone
        let failed = session
            .command("echo")
            .arg("foo")
            .output()
            .await
            .unwrap_err();
        eprintln!("{:?}", failed);
        assert!(matches!(
            failed,
            Error::RemoteProcessTerminated | Error::Disconnected
        ));

        // so does this
        let failed = session
            .command("echo")
            .arg("foo")
            .status()
            .await
            .unwrap_err();
        eprintln!("{:?}", failed);
        assert!(matches!(
            failed,
            Error::RemoteProcessTerminated | Error::Disconnected
        ));

        // the spawned child we're waiting for must also have failed
        let failed = sleeping.wait_with_output().await.unwrap_err();
        eprintln!("{:?}", failed);
        assert!(matches!(failed, Error::RemoteProcessTerminated));

        // check should obviously fail
        let failed = session.check().await.unwrap_err();
        assert!(matches!(failed, Error::Disconnected), "{:?}", failed);

        // Since the ssh multiplex server has exited due to remote sshd process
        // being forcibly killed, `session.close()` should fail here.
        session.close().await.unwrap_err();
    }
}

#[tokio::test]
async fn cannot_resolve() {
    for err in connects_err("bad-host").await {
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

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn remote_socket_forward() {
    let sessions = connects().await;
    for (session, port) in sessions.iter().zip(&[1234, 1233]) {
        let dir = tempdir().unwrap();
        let unix_socket = dir.path().join("unix_socket_listener");

        let output_listener = UnixListener::bind(&unix_socket).unwrap();

        eprintln!("Requesting port forward");
        session
            .request_port_forward(
                ForwardType::Remote,
                Socket::TcpSocket(SocketAddr::new(loopback(), *port)),
                Socket::UnixSocket {
                    path: Cow::Borrowed(&unix_socket),
                },
            )
            .await
            .unwrap();

        eprintln!("Creating remote process");
        let cmd = format!(
            "echo -e '0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n' | nc localhost {} >/dev/stderr",
            port
        );
        let child = session
            .raw_command(cmd)
            .stderr(Stdio::piped())
            .spawn()
            .await
            .unwrap();

        eprintln!("Waiting for connection");
        let (mut output, _addr) = output_listener.accept().await.unwrap();

        eprintln!("Reading");

        const DATA: &[u8] = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n".as_bytes();

        let mut buffer = [0_u8; DATA.len()];
        output.read_exact(&mut buffer).await.unwrap();

        assert_eq!(DATA, &buffer);

        drop(output);
        drop(output_listener);

        eprintln!("Waiting for session to end");
        let output = child.wait_with_output().await.unwrap();
        eprintln!("remote_socket_forward: {:#?}", output);
        assert!(output.status.success());
    }
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn local_socket_forward() {
    let sessions = connects().await;
    for (session, port) in sessions.iter().zip([1433, 1432]) {
        eprintln!("Creating remote process");
        let cmd = format!(
            "echo -e '0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n' | nc -l -p {} >/dev/stderr",
            port
        );
        let child = session
            .raw_command(cmd)
            .stderr(Stdio::piped())
            .spawn()
            .await
            .unwrap();

        sleep(Duration::from_secs(1)).await;

        eprintln!("Requesting port forward");
        let dir = tempdir().unwrap();
        let unix_socket = dir.path().join("unix_socket_forwarded");

        session
            .request_port_forward(
                ForwardType::Local,
                Socket::UnixSocket {
                    path: Cow::Borrowed(&unix_socket),
                },
                Socket::TcpSocket(SocketAddr::new(loopback(), port)),
            )
            .await
            .unwrap();

        eprintln!("Connecting to forwarded socket");
        let mut output = UnixStream::connect(&unix_socket).await.unwrap();

        eprintln!("Reading");

        const DATA: &[u8] = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n".as_bytes();
        let mut buffer = [0_u8; DATA.len()];
        output.read_exact(&mut buffer).await.unwrap();

        assert_eq!(DATA, buffer);

        drop(output);

        eprintln!("Waiting for session to end");
        let output = child.wait_with_output().await.unwrap();
        eprintln!("local_socket_forward: {:#?}", output);
        assert!(output.status.success());
    }
}
