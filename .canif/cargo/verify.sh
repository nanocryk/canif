#!/usr/bin/env bash
# ---
# description: Full acceptance before a commit (fmt check, clippy, tests)
# ---
set -euo pipefail
cd "$CANIF_PROJECT"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
