[![Crates.io](https://img.shields.io/crates/v/openssh.svg)](https://crates.io/crates/openssh)
[![Documentation](https://docs.rs/openssh/badge.svg)](https://docs.rs/openssh/)
[![Codecov](https://codecov.io/github/openssh-rust/openssh/coverage.svg?branch=master)](https://codecov.io/gh/openssh-rust/openssh)

Scriptable SSH through OpenSSH.

This crate wraps the OpenSSH remote login client (`ssh` on most machines), and provides
a convenient mechanism for running commands on remote hosts. Since all commands are executed
through the `ssh` command, all your existing configuration (e.g., in `.ssh/config`) should
continue to work as expected.

The library's API is modeled closely after that of [`std::process::Command`], since `ssh` also
attempts to make the remote process seem as much as possible like a local command.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
