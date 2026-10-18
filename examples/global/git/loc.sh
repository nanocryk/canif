#!/usr/bin/env bash
# ---
# description: Count lines per tracked file of the current git repo, largest last, then the total
# args:
#   exclude:
#     optional: true
#     help: Extended regex of paths to skip, e.g. '\.(lock|png|svg)$'
#   top:
#     type: int
#     min: 1
#     max: 10000
#     optional: true
#     default: 60
#     help: Files listed
# ---
set -uo pipefail
files=$(git ls-files) || exit $?
if [[ -n "${CANIF_EXCLUDE:-}" ]]; then
  files=$(grep -vE -- "$CANIF_EXCLUDE" <<<"$files")
fi
[[ -z $files ]] && { echo "no files"; exit 0; }
counts=$(tr '\n' '\0' <<<"$files" | xargs -0 wc -l | grep -v ' total$' | sort -n)
tail -n "$CANIF_TOP" <<<"$counts"
awk '{ lines += $1; files++ } END { print lines " lines in " files " files" }' <<<"$counts"
