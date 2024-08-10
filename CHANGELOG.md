# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.0](https://github.com/openssh-rust/openssh/compare/v0.10.5...v0.10.6) - 2024-08-10

- Remove dep tokio-pipe (#156)
- Remove deprecated functions (#156)
- Replace `From<tokio::proces::Child*>`
with `TryFrom<tokio::proces::Child*>`, since the converison is falliable (#156)
- Remove `IntoRawFd` for `Child*` since the conversion is falliable (#156)

## [0.10.5](https://github.com/openssh-rust/openssh/compare/v0.10.4...v0.10.5) - 2024-08-10

### Other
- Fix release-plz.yml
- Add missing feature doc for `Session::new*` ([#153](https://github.com/openssh-rust/openssh/pull/153))
- Create release-plz.yml for auto-release ([#151](https://github.com/openssh-rust/openssh/pull/151))
The changelog for this crate is kept in the project's Rust documentation in the changelog module.
