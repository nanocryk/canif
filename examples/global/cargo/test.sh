#!/usr/bin/env bash
# ---
# description: Run the tests, failure details then summed totals (whole workspace unless --package)
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate to test, default is the whole workspace
#   filter:
#     pattern: '[A-Za-z0-9_:]+'
#     optional: true
#     help: Only tests whose name contains this
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

cargo_capture test "${package_args[@]}" --message-format short ${CANIF_FILTER:+"$CANIF_FILTER"}
summarize_tests
exit "$cargo_code"
