#!/bin/bash -ex

cd $(dirname `realpath $0`)

# Start the container
export PUBLIC_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGzHvK2pKtSlZXP9tPYOOBb/xn0IiC9iLMS355AYUPC7'

name=openssh

docker run \
    --name $name \
    --rm \
    -d \
    -p 127.0.0.1:2222:2222 \
    -e 'USER_NAME=test-user' \
    -e 'PUBLIC_KEY' \
    linuxserver/openssh-server:amd64-latest

function cleanup {
    docker stop $name

    # Revert modification to ~/.ssh/known_hosts
    ssh-keygen -R "[127.0.0.1]:2222"
}

trap cleanup EXIT

echo Running the test:
./run_tests.sh
