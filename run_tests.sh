#!/bin/bash

export HOSTNAME=openssh
export TEST_HOST=ssh://test-user@${HOSTNAME}:2222

cd $(dirname `realpath $0`)

echo Test ssh connection
chmod 600 .test-key
ssh -i .test-key -v -p 2222 -l test-user $HOSTNAME \
    -o StrictHostKeyChecking=accept-new whoami

echo Set up ssh agent
eval $(ssh-agent)
cat .test-key | ssh-add -

echo Run tests
for $each in mux_client_impl process_impl; do
    rm -rf $each/control-test $each/config-file-test
done

export RUSTFLAGS='--cfg=ci'
exec cargo test --all-features --workspace --target-dir ./ci-target -- --nocapture
