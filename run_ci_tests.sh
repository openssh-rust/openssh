#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

./start_sshd.sh

export RUSTFLAGS='--cfg=ci'

# Use a different target-dir since RUSTFLAGS='--cfg=ci' seems to
# affect dependencies as well.
#
# Since IDEs usually do not set RUSTFLAGS='--cfg=ci', setting it
# here would cause all the dependencies and openssh to be rebuilt.
#
# Thus it makes run_ci_tests.sh incrediably slow, and it also
# affects IDEs checking, since now the IDEs also need to
# rebuild the crate.
export CARGO_TARGET_DIR=ci-target

cargo hack --feature-powerset check
cargo clippy --all-features

export HOSTNAME=127.0.0.1

echo Set up ssh agent
eval "$(ssh-agent)"
ssh-add - <.test-key

function cleanup {
    ssh-agent -k
}
trap cleanup EXIT

echo Run tests
rm -rf control-test config-file-test .ssh-connection*

echo Running integration test
cargo test \
    --all-features \
    --no-fail-fast \
    -- --test-threads=3 # Use test-threads=3 so that the output is readable
