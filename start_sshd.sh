#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

[ -f "$XDG_RUNTIME_DIR/openssh-rs/sshd_started" ] && exit

# Start the container
export PUBLIC_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGzHvK2pKtSlZXP9tPYOOBb/xn0IiC9iLMS355AYUPC7'
export DOCKER_MODS='linuxserver/mods:openssh-server-ssh-tunnel'

docker run \
    --name openssh \
    --rm \
    -d \
    -p 127.0.0.1:2222:2222 \
    -e 'USER_NAME=test-user' \
    -e DOCKER_MODS \
    -e PUBLIC_KEY \
    linuxserver/openssh-server:amd64-latest

export HOSTNAME=127.0.0.1
chmod 600 .test-key

# Remove 127.0.0.1:2222 from known_hosts
ssh-keygen -R "[127.0.0.1]:2222"
rm -f ~/.ssh/known_hosts.old

echo Waiting for sshd to be up
while ! ssh -i .test-key -v -p 2222 -l test-user $HOSTNAME -o StrictHostKeyChecking=no whoami; do
    sleep 3
done

# Create sshd_started in /tmp/ so that it is auto removed on restart.
mkdir -p "$XDG_RUNTIME_DIR/openssh-rs/"
touch "$XDG_RUNTIME_DIR/openssh-rs/sshd_started"
