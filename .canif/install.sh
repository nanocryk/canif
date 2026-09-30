#!/usr/bin/env bash
# ---
# description: Install the canif binary from this repo into ~/.cargo/bin (cargo install --locked)
# risky: true
# ---
set -uo pipefail
cd "$CANIF_PROJECT"
bin=$(command -v canif) || bin=""
# The running canif.exe cannot be overwritten on Windows, but it can be renamed.
if [[ -n $bin && $bin == *.exe ]]; then
  rm -f "$bin.old" 2>/dev/null
  mv -f "$bin" "$bin.old" || exit $?
fi
cargo install --path . --locked 2>&1 | tail -5
code=${PIPESTATUS[0]}
if [[ $code -ne 0 && -n $bin && ! -e $bin && -e $bin.old ]]; then
  mv "$bin.old" "$bin"
fi
exit "$code"
