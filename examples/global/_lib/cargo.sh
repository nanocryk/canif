# Shared helpers for cargo scripts: run cargo with captured output, print compact summaries.

# Full cargo output, for following a running build; failures reach stdout through the summaries.
cargo_log=$(mktemp)
cargo_code=0
trap 'rm -f "$cargo_log"' EXIT
echo "full cargo output, live while cargo runs, deleted at exit: $(cygpath -m "$cargo_log" 2>/dev/null || echo "$cargo_log")"

# Selects `-p <package>` when CANIF_PACKAGE is set, else the whole workspace.
package_args=(--workspace)
if [[ -n "${CANIF_PACKAGE:-}" ]]; then
  package_args=(-p "$CANIF_PACKAGE")
fi

# Bash before 5.0 (macOS) has no EPOCHREALTIME: fall back to whole seconds.
now_micros() {
  local now=${EPOCHREALTIME:-$SECONDS.000000}
  micros_now=${now//[!0-9]/}
}
now_micros
cargo_started=$micros_now

# Prefixes each line with the seconds since the script started, like canif's own log.
timestamp_lines() {
  local line micros
  while IFS= read -r line || [[ -n $line ]]; do
    now_micros
    micros=$((micros_now - cargo_started))
    printf '[%5d.%ds] %s\n' $((micros / 1000000)) $((micros / 100000 % 10)) "$line"
  done
}

cargo_capture() {
  cargo "$@" 2>&1 | timestamp_lines >"$cargo_log"
  cargo_code=${PIPESTATUS[0]}
}

# The captured output without its timestamps.
cargo_output() {
  sed -E 's/^\[ *[0-9]+\.[0-9]s\] //' "$cargo_log"
}

# One line per diagnostic (needs --message-format short), deduplicated across targets, then counts.
summarize_diagnostics() {
  cargo_output | grep -E '^[^ ]+:[0-9]+:[0-9]+: (error|warning)|^error' | awk '!seen[$0]++'
  local errors warnings
  errors=$(cargo_output | grep -E '^[^ ]+:[0-9]+:[0-9]+: error' | sort -u | wc -l)
  warnings=$(cargo_output | grep -E '^[^ ]+:[0-9]+:[0-9]+: warning' | sort -u | wc -l)
  if [[ $cargo_code -eq 0 && $errors -eq 0 && $warnings -eq 0 ]]; then
    echo "clean"
  else
    echo "$errors errors, $warnings warnings"
  fi
}

# Build errors if tests never ran, else failure details and summed totals.
summarize_tests() {
  # grep -c, not -q: an early exit SIGPIPEs the reader, which pipefail reports as no match.
  if [[ $(cargo_output | grep -c '^test result:') -eq 0 ]]; then
    summarize_diagnostics
    return
  fi
  cargo_output | sed -n '/^failures:$/,/^test result:/p' \
    | grep -vE '^test result:|^note: run with `RUST_BACKTRACE' | cat -s | head -150
  cargo_output | awk '/^test result:/ { passed += $4; failed += $6; ignored += $8; binaries++ }
       END { print passed " passed, " failed " failed, " ignored " ignored, in " binaries " test binaries" }'
}
