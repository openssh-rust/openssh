#!/bin/bash -ex

cd $(dirname `realpath $0`)

# Build the tests
#RUSTFLAGS='--cfg=ci' cargo build --all-features --workspace --tests

# Build the container image
docker build -t openssh-rs-test-env - <Dockerfile || exit

# Start the container
export PUBLIC_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGzHvK2pKtSlZXP9tPYOOBb/xn0IiC9iLMS355AYUPC7'

name=openssh

docker network create $name
docker run \
    --name $name \
    --rm \
    --network $name \
    -d \
    -p 2222:2222 \
    -e 'USER_NAME=test-user' \
    -e 'PUBLIC_KEY' \
    linuxserver/openssh-server:amd64-latest

function cleanup {
    docker stop $name
    docker network rm $name
}

trap cleanup EXIT

echo Running the test:
docker run \
    --name openssh-rs-test-env \
    --rm \
    -it \
    --mount type=bind,src="$PWD",dst=/openssh-rs \
    --network $name \
    openssh-rs-test-env \
    /openssh-rs/run_tests.sh
