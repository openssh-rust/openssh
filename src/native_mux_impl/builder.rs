use super::{Error, Session};
use crate::SessionBuilder;

use std::fs;
use std::str;

pub(crate) async fn just_connect(
    builder: &SessionBuilder,
    destination: &str,
) -> Result<Session, Error> {
    let dir = builder.build_tempdir()?;

    let log = dir.path().join("log");

    let (_child, status) = builder
        .launch_mux_master(destination, &dir, Some(&log))
        .await?;

    if !status.success() {
        let output = fs::read_to_string(log).map_err(Error::Connect)?;

        Err(Error::interpret_ssh_error(&output))
    } else {
        Ok(Session { tempdir: Some(dir) })
    }
}
