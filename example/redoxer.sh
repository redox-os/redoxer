#!/usr/bin/env bash

set -e

redoxer_manifest="$(realpath ../Cargo.toml)"

function redoxer {
	cargo run --release --manifest-path "${redoxer_manifest}" -- "$@"
}

redoxer "$@"
