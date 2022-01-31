#!/bin/bash -ex

cd $(dirname `realpath $0`)

export RUSTFLAGS='--cfg=ci'

cargo check --all-features
exec cargo clippy --all-features
