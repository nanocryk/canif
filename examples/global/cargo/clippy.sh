#!/usr/bin/env bash
# ---
# description: Lint all targets, one line per diagnostic, fails on any warning (whole workspace unless --package)
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate to lint, default is the whole workspace
#   full:
#     type: flag
#     help: Full diagnostics with code snippets and suggestions
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

if [[ -n "${CANIF_FULL:-}" ]]; then
  cargo clippy "${package_args[@]}" --all-targets --keep-going -- -D warnings
  exit $?
fi
cargo_capture clippy "${package_args[@]}" --all-targets --keep-going --message-format short -- -D warnings
summarize_diagnostics
exit "$cargo_code"
