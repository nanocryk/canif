#!/usr/bin/env bash
# ---
# description: Run tests and print only failures and totals (whole workspace unless --package)
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate to test, default is the whole workspace
#   filter:
#     pattern: '[A-Za-z0-9_:]+'
#     optional: true
#     help: Only tests whose name contains this
#   nocapture:
#     type: flag
#     help: Show test stdout, prints the full output
#   full:
#     type: flag
#     help: Print the full cargo output instead of the summary
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

args=(test "${package_args[@]}" --message-format short)
[[ -n "${CANIF_FILTER:-}" ]] && args+=("$CANIF_FILTER")
[[ -n "${CANIF_NOCAPTURE:-}" ]] && args+=(-- --nocapture)

cargo_capture "${args[@]}"
if [[ -n "${CANIF_FULL:-}${CANIF_NOCAPTURE:-}" ]]; then
  cat "$cargo_log"
else
  summarize_tests
fi
exit "$cargo_code"
