#!/usr/bin/env bash
# ---
# description: Lint all targets, warnings are errors
# ---
set -euo pipefail
cd "$CANIF_PROJECT"
cargo clippy --all-targets -- -D warnings
