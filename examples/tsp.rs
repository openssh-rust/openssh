use openssh::*;

fn main() {
    let session =
        Session::connect("ssh://jon@ssh.thesquareplanet.com:222", KnownHosts::Strict).unwrap();

    let ls = session.command("ls").output().unwrap();
    eprintln!(
        "{}",
        String::from_utf8(ls.stdout).expect("server output was not valid UTF-8")
    );

    let whoami = session.command("whoami").output().unwrap();
    assert_eq!(whoami.stdout, b"jon\n");

    session.close().unwrap();
}
