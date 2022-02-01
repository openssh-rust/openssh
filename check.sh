#!/bin/bash -ex

cd $(dirname `realpath $0`)

cargo check --all-features
exec cargo clippy --all-features
