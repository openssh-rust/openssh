#!/bin/bash -ex

export HOSTNAME=127.0.0.1

cd $(dirname `realpath $0`)

echo Test ssh connection
chmod 600 .test-key
ssh -i .test-key -v -p 2222 -l test-user $HOSTNAME \
    -o StrictHostKeyChecking=no whoami

echo Set up ssh agent
eval $(ssh-agent)
cat .test-key | ssh-add -

function cleanup {
    ssh-agent -k
}
trap cleanup EXIT

echo Run tests
rm -rf control-test config-file-test .ssh-connection*

export RUSTFLAGS='--cfg=ci'

echo Running test
cargo test \
    --all-features \
    --no-fail-fast \
    --test openssh \
    -- --test-threads=3 # Use test-threads=3 so that the output is readable
# cargo tarpaulin --forward --all-features
