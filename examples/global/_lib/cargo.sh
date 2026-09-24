# Shared helpers for cargo scripts: run cargo with captured output, print compact summaries.

cargo_log=$(mktemp)
trap 'rm -f "$cargo_log"' EXIT

# Selects `-p <package>` when CANIF_PACKAGE is set, else the whole workspace.
package_args=(--workspace)
if [[ -n "${CANIF_PACKAGE:-}" ]]; then
  package_args=(-p "$CANIF_PACKAGE")
fi

cargo_capture() {
  cargo "$@" >"$cargo_log" 2>&1
  cargo_code=$?
}

# One line per diagnostic (needs --message-format short), deduplicated across targets, then counts.
summarize_diagnostics() {
  grep -E '^[^ ]+:[0-9]+:[0-9]+: (error|warning)|^error' "$cargo_log" | awk '!seen[$0]++'
  local errors warnings
  errors=$(grep -E '^[^ ]+:[0-9]+:[0-9]+: error' "$cargo_log" | sort -u | wc -l)
  warnings=$(grep -E '^[^ ]+:[0-9]+:[0-9]+: warning' "$cargo_log" | sort -u | wc -l)
  if [[ $cargo_code -eq 0 && $errors -eq 0 && $warnings -eq 0 ]]; then
    echo "clean"
  else
    echo "$errors errors, $warnings warnings"
  fi
}

# Build errors if tests never ran, else failure details and summed totals.
summarize_tests() {
  if ! grep -q '^test result:' "$cargo_log"; then
    summarize_diagnostics
    return
  fi
  sed -n '/^failures:$/,/^test result:/p' "$cargo_log" \
    | grep -vE '^test result:|^note: run with `RUST_BACKTRACE' | cat -s | head -150
  awk '/^test result:/ { passed += $4; failed += $6; ignored += $8 }
       END { print passed " passed, " failed " failed, " ignored " ignored" }' "$cargo_log"
}
