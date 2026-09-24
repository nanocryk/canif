#!/usr/bin/env bash
# ---
# description: Type-check all targets
# ---
set -euo pipefail
cd "$CANIF_PROJECT"
cargo check --all-targets
