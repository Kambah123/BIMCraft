#!/bin/sh
# Type-check the core, then the CLI, for wasm32-unknown-unknown.
# CI should run these same commands (a job is in the pull request; it could not be
# written to .github/workflows/ci.yml from this environment, which lacks the workflow scope).
set -eu
rustup target add wasm32-unknown-unknown
cargo check -p bimcraft-core --target wasm32-unknown-unknown
cargo check -p bimcraft-cli --target wasm32-unknown-unknown
