#!/bin/bash

cd $(dirname `realpath $0`)

export PUBLIC_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGzHvK2pKtSlZXP9tPYOOBb/xn0IiC9iLMS355AYUPC7'

name=openssh

[[ $(docker ps -f "name=$name" --format '{{.Names}}') == $name ]] ||
    docker run \
        --name "$name" \
        --rm \
        -d \
        -p 2222:2222 \
        -e 'USER_NAME=test-user' \
        -e 'PUBLIC_KEY' \
        linuxserver/openssh-server:amd64-latest

# Test ssh connection
chmod 600 .test-key
ssh -i .test-key -v -p 2222 -l test-user 127.0.0.1 \
    -o StrictHostKeyChecking=accept-new whoami

# Set up ssh agent
eval $(ssh-agent)
cat .test-key | ssh-add -

# Run tests
rm -rf control-test/ config-file-test/
export RUSTFLAGS='--cfg=ci'

cargo test --workspace -- --nocapture
exit_code=$?
exit $exit_code
