#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

rm "$RUNTIME_DIR/openssh-rs/sshd_started"
rm "$RUNTIME_DIR/openssh-rs/known_hosts"

docker stop openssh
