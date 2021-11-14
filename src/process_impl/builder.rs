use super::super::SessionBuilder;
use super::{Error, Session};

use tokio::io::AsyncReadExt;

pub(crate) async fn just_connect<S: AsRef<str>>(
    builder: &SessionBuilder,
    host: S,
) -> Result<Session, Error> {
    let destination = host.as_ref();

    let dir = builder.build_tempdir()?;

    let (mut child, status) = builder.launch_mux_master(destination, &dir, None).await?;

    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

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
