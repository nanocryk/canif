#!/usr/bin/env bash
# ---
# description: Run tests, optionally only those whose name contains the filter
# args:
#   filter:
#     pattern: '[A-Za-z0-9_:]+'
#     optional: true
#     help: Test name substring, e.g. binding::tests::many
# ---
set -euo pipefail
cd "$CANIF_PROJECT"
cargo test ${CANIF_FILTER:+"$CANIF_FILTER"}
