use super::super::SessionBuilder;
use super::{Error, Session};

use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process;

pub(crate) async fn just_connect<S: AsRef<str>>(
    builder: &SessionBuilder,
    host: S,
) -> Result<Session, Error> {
    let destination = host.as_ref();

    let dir = builder.build_tempdir()?;

    let mut init = process::Command::new("ssh");

    init.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("-S")
        .arg(dir.path().join("master"))
        .arg("-M")
        .arg("-f")
        .arg("-N")
        .arg("-o")
        .arg("ControlPersist=yes")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(builder.known_hosts_check.as_option());

    if let Some(ref timeout) = builder.connect_timeout {
        init.arg("-o").arg(format!("ConnectTimeout={}", timeout));
    }

    if let Some(ref interval) = builder.server_alive_interval {
        init.arg("-o")
            .arg(format!("ServerAliveInterval={}", interval));
    }

    if let Some(ref port) = builder.port {
        init.arg("-p").arg(port);
    }

    if let Some(ref user) = builder.user {
        init.arg("-l").arg(user);
    }

    if let Some(ref k) = builder.keyfile {
        // if the user gives a keyfile, _only_ use that keyfile
        init.arg("-o").arg("IdentitiesOnly=yes");
        init.arg("-i").arg(k);
    }

    if let Some(ref config_file) = builder.config_file {
        init.arg("-F").arg(config_file);
    }

    init.arg(destination);

    // eprintln!("{:?}", init);

    // we spawn and immediately wait, because the process is supposed to fork.
    // note that we cannot use .output, since it _also_ tries to read all of stdout/stderr.
    // if the call _didn't_ error, then the backgrounded ssh client will still hold onto those
    // handles, and it's still running, so those reads will hang indefinitely.
    let mut child = init.spawn().map_err(Error::Connect)?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let status = child.wait().await.map_err(Error::Connect)?;

    if !status.success() {
        let mut err = String::new();
        stderr.read_to_string(&mut err).await.unwrap();
        return Err(Error::interpret_ssh_error(&err));
    }

    Ok(Session {
        ctl: dir,
        addr: String::from(destination),
        terminated: false,
        master: std::sync::Mutex::new(Some((stdout, stderr))),
    })
}
