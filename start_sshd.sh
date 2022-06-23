#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

[ -f "$RUNTIME_DIR/openssh-rs/sshd_started" ] && exit

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

mkdir -p "$RUNTIME_DIR/openssh-rs/"

echo Waiting for sshd to be up
timeout 30 ./wait_for_sshd_start_up.sh

# Add the ip to  known_hosts file
ssh -i .test-key -v -p 2222 -l test-user $HOSTNAME -o StrictHostKeyChecking=no -o UserKnownHostsFile="$RUNTIME_DIR/openssh-rs/known_hosts" whoami

# Create sshd_started in runtime directory so that it is auto removed on restart.
touch "$RUNTIME_DIR/openssh-rs/sshd_started"
