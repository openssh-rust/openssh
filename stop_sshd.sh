#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

rm "$XDG_RUNTIME_DIR/openssh-rs/sshd_started"
rm "$XDG_RUNTIME_DIR/openssh-rs/known_hosts"

docker stop openssh
