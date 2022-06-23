#!/bin/bash

set -euxo pipefail

until ssh-keyscan -p 2222 localhost; do
    sleep 1
done
