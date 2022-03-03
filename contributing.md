Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

### Run integration tests

Requires `docker` and [`cargo-hack`].

Check [getting Docker guide](https://docs.docker.com/get-docker/) on how to install docker,
and use `cargo install cargo-hack` to install [`cargo-hack`].

```
./run_ci_tests.sh
```

It will create a container which runs sshd, setup ssh-agent, and environment variables
that are required to run the integration tests.

It will also test different combination of feature flags to ensure they all compile without error.

[`cargo-hack`]: https://github.com/taiki-e/cargo-hack

### Build documentation

Requires nightly cargo.

To install nightly cargo, run `rustup toolchain install nightly`.

```
./build_doc.sh
```
