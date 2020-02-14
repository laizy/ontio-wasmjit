#!/bin/bash
set -e
set -x

echo $RELEASES_TOKEN

rustc --version

if rustup component add clippy;
then
	cargo clippy --all --all-targets --all-features -- -D warnings
else
	echo 'Skipping clippy';
fi

cargo fmt --all -- --check
cargo build --all
cargo test --all

cd wasmjit-harness
cargo run -- -q

