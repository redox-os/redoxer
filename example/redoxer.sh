#!/usr/bin/env bash

set -e

redoxer_manifest="$(realpath ../Cargo.toml)"

function redoxer {
	cargo run --release --manifest-path "${redoxer_manifest}" -- "$@"
}

# First, install the required tools
redoxer true

# Add the toolchain to your path
source "$HOME/.redoxer/toolchain/env"

xargo build --release --target "${TARGET}"
