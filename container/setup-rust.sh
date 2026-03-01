#!/bin/bash
# Sourced at container start — installs Rust if not already present.
# Use 'return' not 'exit' since this is sourced.

if command -v cargo &>/dev/null; then
    return 0
fi

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --quiet

export PATH="$HOME/.cargo/bin:$PATH"
