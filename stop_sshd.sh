#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

rm -f "$RUNTIME_DIR/openssh-rs/sshd_started" "$RUNTIME_DIR/openssh-rs/known_hosts"

docker stop openssh
