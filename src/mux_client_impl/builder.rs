use super::{Error, Session};
use crate::SessionBuilder;

use std::fs;
use std::io;
use std::process::Stdio;
use std::str;

use tokio::process;

pub(crate) async fn just_connect<S: AsRef<str>>(
    builder: &SessionBuilder,
    host: S,
) -> Result<Session, Error> {
    let destination = host.as_ref();

    let dir = builder.build_tempdir()?;

    let ctl = dir.path().join("master");
    let log = dir.path().join("log");

    let mut init = process::Command::new("ssh");
    init.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("-S")
        .arg(&ctl)
        .arg("-M")
        .arg("-f")
        .arg("-N")
        .arg("-o")
        .arg("ControlPersist=yes")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg(builder.known_hosts_check.as_option())
        .arg("-E")
        .arg(&log);

    if let Some(timeout) = &builder.connect_timeout {
        init.arg("-o").arg(format!("ConnectTimeout={}", timeout));
    }

    if let Some(interval) = &builder.server_alive_interval {
        init.arg("-o")
            .arg(format!("ServerAliveInterval={}", interval));
    }

    if let Some(port) = &builder.port {
        init.arg("-p").arg(port);
    }

    if let Some(user) = &builder.user {
        init.arg("-l").arg(user);
    }

    if let Some(k) = &builder.keyfile {
        // if the user gives a keyfile, _only_ use that keyfile
        init.arg("-o").arg("IdentitiesOnly=yes");
        init.arg("-i").arg(k);
    }

    if let Some(config_file) = &builder.config_file {
        init.arg("-F").arg(config_file);
    }

    init.arg(destination);

    // eprintln!("{:?}", init);

    // we spawn and immediately wait, because the process is supposed to fork.
    // note that we cannot use .output, since it _also_ tries to read all of stdout/stderr.
    // if the call _didn't_ error, then the backgrounded ssh client will still hold onto those
    // handles, and it's still running, so those reads will hang indefinitely.
    let mut child = init.spawn().map_err(Error::Connect)?;
    let status = child.wait().await.map_err(Error::Connect)?;

    if !status.success() {
        let bytes = fs::read(log)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
            .map_err(Error::Connect)?;

        let s = str::from_utf8(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
            .map_err(Error::Connect)?;

        Err(Error::interpret_ssh_error(s))
    } else {
        Ok(Session { tempdir: Some(dir) })
    }
}
