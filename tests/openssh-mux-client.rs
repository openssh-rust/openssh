#![cfg(feature = "mux_client")]

//use core::num::NonZeroU32;
//use core::time::Duration;

//use std::fs;
use std::str;

use once_cell::sync::OnceCell;

//use tokio::net::{TcpListener, TcpStream};
//use tokio::time::sleep;

use openssh::*;

// TODO: how do we test the connection actually _failing_ so that the master reports an error?

fn addr() -> &'static str {
    static ADDR: OnceCell<Option<String>> = OnceCell::new();
    ADDR.get_or_init(|| std::env::var("TEST_HOST").ok())
        .as_deref()
        .unwrap_or("ssh://test-user@127.0.0.1:2222")
}

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn process_exit_on_signal() {
    let session = Session::connect_mux(&addr(), KnownHosts::Accept)
        .await
        .unwrap();

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
    eprintln!("process_exit_on_signal: {:?}", killed);
    assert!(killed.status.success());

    // await that process — this will yield "Disconnected", since the remote process disappeared
    eprintln!("process_exit_on_signal: Waiting for sleeping to exit");
    let failed = sleeping.wait().await.unwrap();
    eprintln!("process_exit_on_signal: {:?}", failed);
    assert!(!failed.success());

    // the connection should still work though
    let _ = session.check().await.unwrap();
}

// These two tests are commented out because linuxserver/openssh-server does not
// allow port forwarding.
//
// However, these functionalities are tested in openssh_mux_client, so I suppose they
// are actually ready to use.
//
//#[tokio::test]
//async fn remote_socket_forward() {
//    let session = Session::connect_mux(&addr(), KnownHosts::Accept).await.unwrap();
//
//    let output_listener = TcpListener::bind(("127.0.0.1", 1234)).await.unwrap();
//
//    eprintln!("Requesting port forward");
//    session
//        .request_port_forward(
//            ForwardType::Remote,
//            &Socket::TcpSocket {
//                port: NonZeroU32::new(9999).unwrap(),
//                host: "127.0.0.1",
//            },
//            &Socket::TcpSocket {
//                port: NonZeroU32::new(1234).unwrap(),
//                host: "127.0.0.1",
//            },
//        )
//        .await
//        .expect(str::from_utf8(&read_log(&session)).unwrap());
//
//    eprintln!("Creating remote process");
//    let cmd = "echo -e '0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n' | nc localhost 9999 >/dev/stderr";
//    let child = session
//        .raw_command(cmd)
//        .stderr(Stdio::piped())
//        .spawn()
//        .await
//        .unwrap();
//
//    eprintln!("Waiting for connection");
//    let (mut output, _addr) = output_listener.accept().await.unwrap();
//
//    eprintln!("Reading");
//
//    const DATA: &[u8] = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n".as_bytes();
//
//    let mut buffer = [0 as u8; DATA.len()];
//    output.read_exact(&mut buffer).await.unwrap();
//
//    assert_eq!(DATA, &buffer);
//
//    drop(output);
//    drop(output_listener);
//
//    eprintln!("Waiting for session to end");
//    let output = child.wait_with_output().await.unwrap();
//    eprintln!("remote_socket_forward: {:#?}", output);
//    assert!(output.status.success());
//}
//
//#[tokio::test]
//async fn local_socket_forward() {
//    let session = Session::connect_mux(&addr(), KnownHosts::Accept).await.unwrap();
//
//    eprintln!("Creating remote process");
//    let cmd = "echo -e '0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n' | nc -l -p 1433 >/dev/stderr";
//    let child = session
//        .raw_command(cmd)
//        .stderr(Stdio::piped())
//        .spawn()
//        .await
//        .unwrap();
//
//    sleep(Duration::from_secs(1)).await;
//
//    eprintln!("Requesting port forward");
//    session
//        .request_port_forward(
//            ForwardType::Local,
//            &Socket::TcpSocket {
//                port: NonZeroU32::new(1235).unwrap(),
//                host: "127.0.0.1",
//            },
//            &Socket::TcpSocket {
//                port: NonZeroU32::new(1433).unwrap(),
//                host: "127.0.0.1",
//            },
//        )
//        .await
//        .unwrap();
//
//    eprintln!("Connecting to forwarded socket");
//    let mut output = TcpStream::connect(("127.0.0.1", 1235)).await.unwrap();
//
//    eprintln!("Reading");
//
//    const DATA: &[u8] = "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n".as_bytes();
//    let mut buffer = [0 as u8; DATA.len()];
//    output
//        .read_exact(&mut buffer)
//        .await
//        .expect(str::from_utf8(&read_log(&session)).unwrap());
//
//    assert_eq!(DATA, buffer);
//
//    drop(output);
//
//    eprintln!("Waiting for session to end");
//    let output = child.wait_with_output().await.unwrap();
//    eprintln!("local_socket_forward: {:#?}", output);
//    assert!(output.status.success());
//}
