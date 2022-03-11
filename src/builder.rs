use super::{Error, Session};

#[cfg(feature = "process-mux")]
use super::process_impl;

#[cfg(feature = "native-mux")]
use super::native_mux_impl;

use std::borrow::Cow;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str;

use dirs::state_dir;
use once_cell::sync::OnceCell;
use tempfile::{Builder, TempDir};
use tokio::process;

/// The returned `&'static Path` can be coreced to any lifetime.
fn get_default_control_dir<'a>() -> Result<&'a Path, Error> {
    static DEFAULT_CONTROL_DIR: OnceCell<Option<Box<Path>>> = OnceCell::new();

    DEFAULT_CONTROL_DIR
        .get_or_try_init(|| {
            if let Some(state_dir) = state_dir() {
                fs::create_dir_all(&state_dir).map_err(Error::Connect)?;

                Ok(Some(state_dir.into_boxed_path()))
            } else {
                Ok(None)
            }
        })
        .map(|default_control_dir| {
            default_control_dir
                .as_deref()
                .unwrap_or_else(|| Path::new("./"))
        })
}

/// Build a [`Session`] with options.
#[derive(Debug, Clone)]
pub struct SessionBuilder {
    user: Option<String>,
    port: Option<String>,
    keyfile: Option<PathBuf>,
    connect_timeout: Option<String>,
    server_alive_interval: Option<u64>,
    known_hosts_check: KnownHosts,
    control_dir: Option<PathBuf>,
    config_file: Option<PathBuf>,
    compression: Option<bool>,
    user_known_hosts_file: Option<Box<Path>>,
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self {
            user: None,
            port: None,
            keyfile: None,
            connect_timeout: None,
            server_alive_interval: None,
            known_hosts_check: KnownHosts::Add,
            control_dir: None,
            config_file: None,
            compression: None,
            user_known_hosts_file: None,
        }
    }
}

impl SessionBuilder {
    /// Set the ssh user (`ssh -l`).
    ///
    /// Defaults to `None`.
    pub fn user(&mut self, user: String) -> &mut Self {
        self.user = Some(user);
        self
    }

    /// Set the port to connect on (`ssh -p`).
    ///
    /// Defaults to `None`.
    pub fn port(&mut self, port: u16) -> &mut Self {
        self.port = Some(format!("{}", port));
        self
    }

    /// Set the keyfile to use (`ssh -i`).
    ///
    /// Defaults to `None`.
    pub fn keyfile(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.keyfile = Some(p.as_ref().to_path_buf());
        self
    }

    /// See [`KnownHosts`].
    ///
    /// Default `KnownHosts::Add`.
    pub fn known_hosts_check(&mut self, k: KnownHosts) -> &mut Self {
        self.known_hosts_check = k;
        self
    }

    /// Set the connection timeout (`ssh -o ConnectTimeout`).
    ///
    /// This value is specified in seconds. Any sub-second duration remainder will be ignored.
    /// Defaults to `None`.
    pub fn connect_timeout(&mut self, d: std::time::Duration) -> &mut Self {
        self.connect_timeout = Some(d.as_secs().to_string());
        self
    }

    /// Set the timeout interval after which if no data has been received from the server, ssh
    /// will request a response from the server (`ssh -o ServerAliveInterval`).
    ///
    /// This value is specified in seconds. Any sub-second duration remainder will be ignored.
    /// Defaults to `None`.
    pub fn server_alive_interval(&mut self, d: std::time::Duration) -> &mut Self {
        self.server_alive_interval = Some(d.as_secs());
        self
    }

    /// Set the directory in which the temporary directory containing the control socket will
    /// be created.
    ///
    /// If not set, `./` will be used (the current directory).
    #[cfg(not(windows))]
    #[cfg_attr(docsrs, doc(cfg(not(windows))))]
    pub fn control_directory(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.control_dir = Some(p.as_ref().to_path_buf());
        self
    }

    /// Set an alternative per-user configuration file.
    ///
    /// By default, ssh uses `~/.ssh/config`. This is equivalent to `ssh -F <p>`.
    ///
    /// Defaults to `None`.
    pub fn config_file(&mut self, p: impl AsRef<Path>) -> &mut Self {
        self.config_file = Some(p.as_ref().to_path_buf());
        self
    }

    /// Enable or disable compression (including stdin, stdout, stderr, data
    /// for forwarded TCP and unix-domain connections, sftp and scp
    /// connections).
    ///
    /// Note that the ssh server can forcibly disable the compression.
    ///
    /// By default, ssh uses configure value set in `~/.ssh/config`.
    ///
    /// If `~/.ssh/config` does not enable compression, then it is disabled
    /// by default.
    pub fn compression(&mut self, compression: bool) -> &mut Self {
        self.compression = Some(compression);
        self
    }

    /// Specify the path to the `known_hosts` file.
    ///
    /// The path provided may use tilde notation (`~`) to refer to the user's
    /// home directory.
    ///
    /// The default is `~/.ssh/known_hosts` and `~/.ssh/known_hosts2`.
    pub fn user_known_hosts_file(&mut self, user_known_hosts_file: impl AsRef<Path>) -> &mut Self {
        self.user_known_hosts_file =
            Some(user_known_hosts_file.as_ref().to_owned().into_boxed_path());
        self
    }

    /// Connect to the host at the given `host` over SSH using process impl, which will
    /// spawn a new ssh process for each `Child` created.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    /// A username or port that is specified in the connection string overrides the one set in the
    /// builder (but does not change the builder).
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    #[cfg(feature = "process-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "process-mux")))]
    pub async fn connect<S: AsRef<str>>(&self, destination: S) -> Result<Session, Error> {
        let destination = destination.as_ref();
        let (builder, destination) = self.resolve(destination);
        let tempdir = builder.launch_master(destination).await?;
        Ok(process_impl::Session::new(tempdir, destination).into())
    }

    /// Connect to the host at the given `host` over SSH using native mux, which will
    /// create a new local socket connection for each `Child` created.
    ///
    /// See the crate-level documentation for more details on the difference between native and process-based mux.
    ///
    /// The format of `destination` is the same as the `destination` argument to `ssh`. It may be
    /// specified as either `[user@]hostname` or a URI of the form `ssh://[user@]hostname[:port]`.
    /// A username or port that is specified in the connection string overrides the one set in the
    /// builder (but does not change the builder).
    ///
    /// If connecting requires interactive authentication based on `STDIN` (such as reading a
    /// password), the connection will fail. Consider setting up keypair-based authentication
    /// instead.
    #[cfg(feature = "native-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "native-mux")))]
    pub async fn connect_mux<S: AsRef<str>>(&self, destination: S) -> Result<Session, Error> {
        let destination = destination.as_ref();
        let (builder, destination) = self.resolve(destination);
        let tempdir = builder.launch_master(destination).await?;
        Ok(native_mux_impl::Session::new(tempdir).into())
    }

    fn resolve<'a, 'b>(&'a self, mut destination: &'b str) -> (Cow<'a, Self>, &'b str) {
        // the "new" ssh://user@host:port form is not supported by all versions of ssh,
        // so we always translate it into the option form.
        let mut user = None;
        let mut port = None;
        if destination.starts_with("ssh://") {
            destination = &destination[6..];
            if let Some(at) = destination.find('@') {
                // specified a username -- extract it:
                user = Some(&destination[..at]);
                destination = &destination[(at + 1)..];
            }
            if let Some(colon) = destination.rfind(':') {
                let p = &destination[(colon + 1)..];
                if let Ok(p) = p.parse() {
                    // user specified a port -- extract it:
                    port = Some(p);
                    destination = &destination[..colon];
                }
            }
        }

        if user.is_none() && port.is_none() {
            return (Cow::Borrowed(self), destination);
        }

        let mut with_overrides = self.clone();
        if let Some(user) = user {
            with_overrides.user(user.to_owned());
        }

        if let Some(port) = port {
            with_overrides.port(port);
        }

        (Cow::Owned(with_overrides), destination)
    }

    async fn launch_master(&self, destination: &str) -> Result<TempDir, Error> {
        let socketdir = if let Some(socketdir) = self.control_dir.as_ref() {
            socketdir
        } else {
            get_default_control_dir()?
        };

        let dir = Builder::new()
            .prefix(".ssh-connection")
            .tempdir_in(socketdir)
            .map_err(Error::Master)?;

        let log = dir.path().join("log");

        let mut init = process::Command::new("ssh");

        init.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-E")
            .arg(&log)
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
            .arg(self.known_hosts_check.as_option());

        if let Some(ref timeout) = self.connect_timeout {
            init.arg("-o").arg(format!("ConnectTimeout={}", timeout));
        }

        if let Some(ref interval) = self.server_alive_interval {
            init.arg("-o")
                .arg(format!("ServerAliveInterval={}", interval));
        }

        if let Some(ref port) = self.port {
            init.arg("-p").arg(port);
        }

        if let Some(ref user) = self.user {
            init.arg("-l").arg(user);
        }

        if let Some(ref k) = self.keyfile {
            // if the user gives a keyfile, _only_ use that keyfile
            init.arg("-o").arg("IdentitiesOnly=yes");
            init.arg("-i").arg(k);
        }

        if let Some(ref config_file) = self.config_file {
            init.arg("-F").arg(config_file);
        }

        if let Some(compression) = self.compression {
            let arg = if compression { "yes" } else { "no" };

            init.arg("-o").arg(format!("Compression={}", arg));
        }

        if let Some(user_known_hosts_file) = &self.user_known_hosts_file {
            let mut option: OsString = "UserKnownHostsFile=".into();
            option.push(&**user_known_hosts_file);
            init.arg("-o").arg(option);
        }

        init.arg(destination);

        // we spawn and immediately wait, because the process is supposed to fork.
        let status = init.status().await.map_err(Error::Connect)?;

        if !status.success() {
            let output = fs::read_to_string(log).map_err(Error::Connect)?;

            Err(Error::interpret_ssh_error(&output))
        } else {
            Ok(dir)
        }
    }
}

/// Specifies how the host's key fingerprint should be handled.
#[derive(Debug, Clone)]
pub enum KnownHosts {
    /// The host's fingerprint must match what is in the known hosts file.
    ///
    /// If the host is not in the known hosts file, the connection is rejected.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=yes`.
    Strict,
    /// Strict, but if the host is not already in the known hosts file, it will be added.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=accept-new`.
    Add,
    /// Accept whatever key the server provides and add it to the known hosts file.
    ///
    /// This corresponds to `ssh -o StrictHostKeyChecking=no`.
    Accept,
}

impl KnownHosts {
    fn as_option(&self) -> &'static str {
        match *self {
            KnownHosts::Strict => "StrictHostKeyChecking=yes",
            KnownHosts::Add => "StrictHostKeyChecking=accept-new",
            KnownHosts::Accept => "StrictHostKeyChecking=no",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionBuilder;

    #[test]
    fn resolve() {
        let b = SessionBuilder::default();
        let (b, d) = b.resolve("ssh://test-user@127.0.0.1:2222");
        assert_eq!(b.port.as_deref(), Some("2222"));
        assert_eq!(b.user.as_deref(), Some("test-user"));
        assert_eq!(d, "127.0.0.1");

        let b = SessionBuilder::default();
        let (b, d) = b.resolve("ssh://test-user@opensshtest:2222");
        assert_eq!(b.port.as_deref(), Some("2222"));
        assert_eq!(b.user.as_deref(), Some("test-user"));
        assert_eq!(d, "opensshtest");

        let b = SessionBuilder::default();
        let (b, d) = b.resolve("ssh://opensshtest:2222");
        assert_eq!(b.port.as_deref(), Some("2222"));
        assert_eq!(b.user.as_deref(), None);
        assert_eq!(d, "opensshtest");

        let b = SessionBuilder::default();
        let (b, d) = b.resolve("ssh://test-user@opensshtest");
        assert_eq!(b.port.as_deref(), None);
        assert_eq!(b.user.as_deref(), Some("test-user"));
        assert_eq!(d, "opensshtest");

        let b = SessionBuilder::default();
        let (b, d) = b.resolve("ssh://opensshtest");
        assert_eq!(b.port.as_deref(), None);
        assert_eq!(b.user.as_deref(), None);
        assert_eq!(d, "opensshtest");

        let b = SessionBuilder::default();
        let (b, d) = b.resolve("opensshtest");
        assert_eq!(b.port.as_deref(), None);
        assert_eq!(b.user.as_deref(), None);
        assert_eq!(d, "opensshtest");
    }
}
