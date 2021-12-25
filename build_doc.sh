#!/bin/sh

export RUSTDOCFLAGS="--cfg docsrs"
exec cargo +nightly doc --all-features
