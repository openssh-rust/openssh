#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

[ -z ${XDG_RUNTIME_DIR+x} ] && export XDG_RUNTIME_DIR=/tmp

rm "$XDG_RUNTIME_DIR/openssh-rs/sshd_started"
rm "$XDG_RUNTIME_DIR/openssh-rs/known_hosts"

docker stop openssh
