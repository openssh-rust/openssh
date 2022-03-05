use std::fmt;
use std::io;

/// Errors that occur when interacting with a remote process.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The master connection failed.
    Master(io::Error),

    /// Failed to establish initial connection to the remote host.
    Connect(io::Error),

    /// Failed to run the `ssh` command locally.
    #[cfg(feature = "process-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "process-mux")))]
    Ssh(io::Error),

    /// Failed to connect to the ssh multiplex server.
    #[cfg(feature = "native-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "native-mux")))]
    SshMux(openssh_mux_client::Error),

    /// Invalid command that contains null byte.
    #[cfg(feature = "native-mux")]
    #[cfg_attr(docsrs, doc(cfg(feature = "native-mux")))]
    InvalidCommand,

    /// The remote process failed.
    Remote(io::Error),

    /// The connection to the remote host was severed.
    ///
    /// Note that for the process impl, this is a best-effort error, and it _may_ instead
    /// signify that the remote process exited with an error code of 255.
    ///
    /// You should call [`Session::check`](crate::Session::check) to verify if you get
    /// this error back.
    Disconnected,

    /// Remote process is terminated.
    ///
    /// It is likely to be that the process is terminated by signal.
    ///
    /// **NOTE that due to a fundamental design flaw in ssh multiplex protocol,
    /// there is no way to tell `RemoteProcessTerminated` from `Disconnect`.**
    ///
    /// If you really need to identify `Disconnect`, you can call `session.check()`
    /// after `wait()` returns `RemoteProcessTerminated`, however the ssh multiplex master
    /// could exit right after `wait()`, meaning the remote process actually is terminated
    /// instead of `Disconnect`ed.
    ///
    /// It is thus recommended to create your own workaround for your particular use cases.
    RemoteProcessTerminated,

    /// Failed to remove temporary dir where ssh socket and output is stored.
    Cleanup(io::Error),

    /// IO Error when creating/reading/writing from ChildStdin, ChildStdout, ChildStderr.
    ChildIo(io::Error),
}

#[cfg(feature = "native-mux")]
impl From<openssh_mux_client::Error> for Error {
    fn from(err: openssh_mux_client::Error) -> Self {
        use io::ErrorKind;

        match &err {
            openssh_mux_client::Error::IOError(ioerr) => match ioerr.kind() {
                ErrorKind::NotFound
                | ErrorKind::ConnectionReset
                // If the listener of a unix socket exits without removing the socket
                // file, then attempt to connect to the file results in
                // `ConnectionRefused`.
                | ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected => Error::Disconnected,

                _ => Error::SshMux(err),
            },
            _ => Error::SshMux(err),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Master(_) => write!(f, "the master connection failed"),
            Error::Connect(_) => write!(f, "failed to connect to the remote host"),

            #[cfg(feature = "process-mux")]
            Error::Ssh(_) => write!(f, "the local ssh command could not be executed"),

            Error::Remote(_) => write!(f, "the remote command could not be executed"),
            Error::Disconnected => write!(f, "the connection was terminated"),
            Error::Cleanup(_) => write!(f, "failed to remove temporary ssh session directory"),
            Error::ChildIo(_) => {
                write!(f, "failure while accessing standard I/O of remote process")
            }

            Error::RemoteProcessTerminated => write!(f, "the remote process has terminated"),

            #[cfg(feature = "native-mux")]
            Error::SshMux(_) => write!(f, "failed to connect to the ssh multiplex server"),

            #[cfg(feature = "native-mux")]
            Error::InvalidCommand => write!(f, "invalid command: Command contains null byte."),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::Master(ref e)
            | Error::Connect(ref e)
            | Error::Remote(ref e)
            | Error::Cleanup(ref e)
            | Error::ChildIo(ref e) => Some(e),

            Error::RemoteProcessTerminated | Error::Disconnected => None,

            #[cfg(feature = "native-mux")]
            Error::InvalidCommand => None,

            #[cfg(feature = "process-mux")]
            Error::Ssh(ref e) => Some(e),

            #[cfg(feature = "native-mux")]
            Error::SshMux(ref e) => Some(e),
        }
    }
}

impl Error {
    pub(crate) fn interpret_ssh_error(stderr: &str) -> Self {
        // we want to turn the string-only ssh error into something a little more "handleable".
        // we do this by trying to interpret the output from `ssh`. this is error-prone, but
        // the best we can do. if you find ways to impove this, even just through heuristics,
        // please file an issue or PR :)
        //
        // format is:
        //
        //     ssh: ssh error: io error
        let mut stderr = stderr.trim();
        stderr = stderr.strip_prefix("ssh: ").unwrap_or(stderr);
        if stderr.starts_with("Warning: Permanently added ") {
            // added to hosts file -- let's ignore that message
            stderr = stderr.split_once('\n').map(|x| x.1.trim()).unwrap_or("");
        }
        let mut kind = io::ErrorKind::ConnectionAborted;
        let mut err = stderr.splitn(2, ": ");
        if let Some(ssh_error) = err.next() {
            if ssh_error.starts_with("Could not resolve") {
                // match what `std` gives: https://github.com/rust-lang/rust/blob/a5de254862477924bcd8b9e1bff7eadd6ffb5e2a/src/libstd/sys/unix/net.rs#L40
                // we _could_ match on "Name or service not known" from io_error,
                // but my guess is that the ssh error is more stable.
                kind = io::ErrorKind::Other;
            }

            if let Some(io_error) = err.next() {
                match io_error {
                    "Network is unreachable" => {
                        kind = io::ErrorKind::Other;
                    }
                    "Connection refused" => {
                        kind = io::ErrorKind::ConnectionRefused;
                    }
                    e if ssh_error.starts_with("connect to host")
                        && e == "Connection timed out" =>
                    {
                        kind = io::ErrorKind::TimedOut;
                    }
                    e if ssh_error.starts_with("connect to host") && e == "Operation timed out" => {
                        // this is the macOS version of "connection timed out"
                        kind = io::ErrorKind::TimedOut;
                    }
                    e if ssh_error.starts_with("connect to host") && e == "Permission denied" => {
                        // this is the macOS version of "network is unreachable".
                        kind = io::ErrorKind::Other;
                    }
                    e if e.contains("Permission denied (") => {
                        kind = io::ErrorKind::PermissionDenied;
                    }
                    _ => {}
                }
            }
        }

        // NOTE: we may want to provide more structured connection errors than just io::Error?
        // NOTE: can we re-use this method for non-connect cases?
        Error::Connect(io::Error::new(kind, stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::{io, Error};

    #[test]
    fn parse_error() {
        let err = "ssh: Warning: Permanently added \'login.csail.mit.edu,128.52.131.0\' (ECDSA) to the list of known hosts.\r\nopenssh-tester@login.csail.mit.edu: Permission denied (publickey,gssapi-keyex,gssapi-with-mic,password,keyboard-interactive).";
        let err = Error::interpret_ssh_error(err);
        let target = io::Error::new(io::ErrorKind::PermissionDenied, "openssh-tester@login.csail.mit.edu: Permission denied (publickey,gssapi-keyex,gssapi-with-mic,password,keyboard-interactive).");
        if let Error::Connect(e) = err {
            assert_eq!(e.kind(), target.kind());
            assert_eq!(format!("{}", e), format!("{}", target));
        } else {
            unreachable!("{:?}", err);
        }
    }

    #[test]
    fn error_sanity() {
        use std::error::Error as _;

        let ioe = || io::Error::new(io::ErrorKind::Other, "test");
        let expect = ioe();

        let e = Error::Master(ioe());
        assert!(!format!("{}", e).is_empty());
        let e = e
            .source()
            .expect("source failed")
            .downcast_ref::<io::Error>()
            .expect("source not io");
        assert_eq!(e.kind(), expect.kind());
        assert_eq!(format!("{}", e), format!("{}", expect));

        let e = Error::Connect(ioe());
        assert!(!format!("{}", e).is_empty());
        let e = e
            .source()
            .expect("source failed")
            .downcast_ref::<io::Error>()
            .expect("source not io");
        assert_eq!(e.kind(), expect.kind());
        assert_eq!(format!("{}", e), format!("{}", expect));

        #[cfg(feature = "process-mux")]
        {
            let e = Error::Ssh(ioe());
            assert!(!format!("{}", e).is_empty());
            let e = e
                .source()
                .expect("source failed")
                .downcast_ref::<io::Error>()
                .expect("source not io");
            assert_eq!(e.kind(), expect.kind());
            assert_eq!(format!("{}", e), format!("{}", expect));
        }

        let e = Error::Remote(ioe());
        assert!(!format!("{}", e).is_empty());
        let e = e
            .source()
            .expect("source failed")
            .downcast_ref::<io::Error>()
            .expect("source not io");
        assert_eq!(e.kind(), expect.kind());
        assert_eq!(format!("{}", e), format!("{}", expect));

        let e = Error::Disconnected;
        assert!(!format!("{}", e).is_empty());
        assert!(e.source().is_none());
    }
}
