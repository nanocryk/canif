#!/usr/bin/env bash
# ---
# description: Delete local git branches already merged into the default branch (never the current one)
# risky: true
# args:
#   into:
#     pattern: '[A-Za-z0-9._/-]+'
#     optional: true
#     help: Branch to compare against, default is origin's HEAD, else main or master
#   dry-run:
#     type: flag
#     help: Only list the branches that would be deleted
# ---
set -uo pipefail

base=${CANIF_INTO:-}
if [[ -z $base ]]; then
  base=$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null)
  base=${base#origin/}
fi
if [[ -z $base ]]; then
  for candidate in main master; do
    git show-ref --verify --quiet "refs/heads/$candidate" && { base=$candidate; break; }
  done
fi
[[ -z $base ]] && { echo "no default branch found, pass --into"; exit 1; }

current=$(git branch --show-current) || exit $?
branches=$(git branch --merged "$base" --format='%(refname:short)' | grep -vxF -e "$base" -e "$current")
[[ -z $branches ]] && { echo "no branches merged into $base"; exit 0; }

if [[ -n ${CANIF_DRY_RUN:-} ]]; then
  echo "would delete, merged into $base:"
  echo "$branches"
  exit 0
fi
# Unquoted on purpose: branch names cannot contain whitespace.
git branch -D $branches
