#!/bin/bash -ex

cd $(dirname `realpath $0`)

# Start the container
export PUBLIC_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGzHvK2pKtSlZXP9tPYOOBb/xn0IiC9iLMS355AYUPC7'
export DOCKER_MODS='linuxserver/mods:openssh-server-ssh-tunnel'

name=openssh

docker run \
    --name $name \
    --rm \
    -d \
    -p 127.0.0.1:2222:2222 \
    -e 'USER_NAME=test-user' \
    -e DOCKER_MODS \
    -e PUBLIC_KEY \
    linuxserver/openssh-server:amd64-latest

function cleanup {
    docker stop $name

    # Revert modification to ~/.ssh/known_hosts
    ssh-keygen -R "[127.0.0.1]:2222"

    ssh-agent -k
}
trap cleanup EXIT

export RUSTFLAGS='--cfg=ci'

# Wait for docker mod to be installed and sshd starts up
cargo clippy --all-features

echo Running the test:

export HOSTNAME=127.0.0.1
chmod 600 .test-key

echo Waiting for sshd to be up
while ! ssh -i .test-key -v -p 2222 -l test-user $HOSTNAME -o StrictHostKeyChecking=no whoami; do
    sleep 3
done

echo Set up ssh agent
eval $(ssh-agent)
cat .test-key | ssh-add -

echo Run tests
rm -rf control-test config-file-test .ssh-connection*

for features in process native-mux process,native-mux; do
    echo Running test with features = $features
    cargo test \
        --no-default-features \
        --features "$features" \
        --no-fail-fast \
        --test openssh \
        -- --test-threads=3 # Use test-threads=3 so that the output is readable
done
