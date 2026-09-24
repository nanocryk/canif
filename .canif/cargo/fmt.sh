#!/usr/bin/env bash
# ---
# description: Format the crate, or only report unformatted files with --check
# args:
#   check:
#     type: flag
#     help: Report without writing
# ---
set -euo pipefail
cd "$CANIF_PROJECT"
cargo fmt ${CANIF_CHECK:+--check}
