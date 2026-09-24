#!/usr/bin/env bash
# ---
# description: Pre-commit gate, stops at the first failing step (fmt check, clippy with warnings denied, tests)
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate for clippy and tests, default is the whole workspace
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

echo "== fmt"
if ! cargo fmt --all --check --message-format short; then
  exit 1
fi
echo "formatted"

echo "== clippy"
cargo_capture clippy "${package_args[@]}" --all-targets --message-format short -- -D warnings
summarize_diagnostics
[[ $cargo_code -eq 0 ]] || exit "$cargo_code"

echo "== test"
cargo_capture test "${package_args[@]}" --message-format short
summarize_tests
exit "$cargo_code"
