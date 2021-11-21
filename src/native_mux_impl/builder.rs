use super::{Error, Session};
use crate::SessionBuilder;

use std::fs;
use std::io;
use std::str;

pub(crate) async fn just_connect<S: AsRef<str>>(
    builder: &SessionBuilder,
    host: S,
) -> Result<Session, Error> {
    let destination = host.as_ref();

    let dir = builder.build_tempdir()?;

    let log = dir.path().join("log");

    let (_child, status) = builder
        .launch_mux_master(destination, &dir, Some(&log))
        .await?;

    if !status.success() {
        let bytes = fs::read(log).map_err(Error::Connect)?;

        let s = str::from_utf8(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
            .map_err(Error::Connect)?;

        Err(Error::interpret_ssh_error(s))
    } else {
        Ok(Session { tempdir: Some(dir) })
    }
}
