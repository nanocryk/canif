#!/usr/bin/env bash
# ---
# description: Format the whole workspace, or with --check only list unformatted files; --lint then runs clippy with warnings denied
# args:
#   check:
#     type: flag
#     help: List files needing formatting without writing
#   lint:
#     type: flag
#     help: Then run clippy on all targets, one line per diagnostic, failing on any warning
# ---
set -uo pipefail

if [[ -n "${CANIF_CHECK:-}" ]]; then
  cargo fmt --all --check --message-format short || exit 1
else
  cargo fmt --all || exit $?
fi
echo "formatted"
[[ -z "${CANIF_LINT:-}" ]] && exit 0

source "$CANIF_DIR/_lib/cargo.sh"
cargo_capture clippy --workspace --all-targets --keep-going --message-format short -- -D warnings
summarize_diagnostics
exit "$cargo_code"
