#!/bin/bash -ex

cd $(dirname `realpath $0`)

rm sshd_started

docker stop openssh

# Revert modification to ~/.ssh/known_hosts
ssh-keygen -R "[127.0.0.1]:2222"
