#!/bin/bash

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

echo Running the test:
docker run \
    --name openssh-rs-test-env \
    --rm \
    -it \
    --mount type=bind,src="$PWD",dst=/openssh-rs \
    --network $name \
    openssh-rs-test-env \
    /openssh-rs/run_tests.sh
exit_code=$?
docker rm -f openssh-rs-test-env 2>/dev/null

if [ $exit_code -ne 0 ]; then
    echo Test failed, here\'s the log of sshd:

    #docker logs $(docker ps | grep openssh-server | awk '{print $1}')
fi

docker stop $name
docker network rm $name

exit $exit_code
