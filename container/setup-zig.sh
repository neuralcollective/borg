#!/bin/bash
# Setup script for borg pipeline agents — installs zig
set -e

if command -v zig &>/dev/null; then
    exit 0
fi

curl -fsSL https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz \
    | tar -xJ -C /tmp
export PATH="/tmp/zig-x86_64-linux-0.14.1:$PATH"
