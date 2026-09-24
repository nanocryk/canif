#!/usr/bin/env bash
# ---
# description: Type-check all targets, one line per diagnostic (whole workspace unless --package)
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate to check, default is the whole workspace
#   full:
#     type: flag
#     help: Full rustc diagnostics with code snippets
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

if [[ -n "${CANIF_FULL:-}" ]]; then
  cargo check "${package_args[@]}" --all-targets
  exit $?
fi
cargo_capture check "${package_args[@]}" --all-targets --message-format short
summarize_diagnostics
exit "$cargo_code"
