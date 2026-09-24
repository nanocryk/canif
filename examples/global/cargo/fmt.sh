#!/usr/bin/env bash
# ---
# description: Format the whole workspace, or with --check only list unformatted files
# args:
#   check:
#     type: flag
#     help: List files needing formatting without writing
# ---
set -uo pipefail

if [[ -n "${CANIF_CHECK:-}" ]]; then
  if cargo fmt --all --check --message-format short; then
    echo "formatted"
  else
    exit 1
  fi
else
  cargo fmt --all && echo "formatted"
fi
