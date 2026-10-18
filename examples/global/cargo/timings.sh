#!/usr/bin/env bash
# ---
# description: Build with --timings and list the slowest crates, plus the path to cargo's HTML report
# args:
#   package:
#     pattern: '[A-Za-z0-9_-]+'
#     optional: true
#     help: Crate to build, default is the whole workspace
#   tests:
#     type: flag
#     help: Build the test binaries too (cargo test --no-run), not just the libs and bins
#   top:
#     pattern: '[0-9]+'
#     optional: true
#     help: How many units to list, default 10
# ---
set -uo pipefail
source "$CANIF_DIR/_lib/cargo.sh"

if [[ -n "${CANIF_TESTS:-}" ]]; then
  cargo_capture test "${package_args[@]}" --no-run --timings --message-format short
else
  cargo_capture build "${package_args[@]}" --timings --message-format short
fi
if [[ $cargo_code -ne 0 ]]; then
  summarize_diagnostics
  exit "$cargo_code"
fi

# `py` is the Windows launcher, where `python3` may be a store stub.
python=python3
command -v py >/dev/null && python=py

target_dir=$(cargo metadata --format-version 1 --no-deps | "$python" -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
report="$target_dir/cargo-timings/cargo-timing.html"

# The report embeds every unit as `const UNIT_DATA = [...];`; `duration` covers the unit's
# whole compile, link included, `rmeta_time` the part before codegen.
CANIF_REPORT="$report" "$python" - <<'EOF'
import json, os, re

report = os.environ["CANIF_REPORT"]
top = int(os.environ.get("CANIF_TOP") or 10)
html = open(report, encoding="utf-8").read()
match = re.search(r"const UNIT_DATA = (\[.*?\]);\n", html, re.S)
if not match:
    raise SystemExit(f"no UNIT_DATA in {report}")
units = json.loads(match.group(1))
wall = max(u["start"] + u["duration"] for u in units)
print(f"{len(units)} units, {wall:.1f}s wall")
for u in sorted(units, key=lambda u: u["duration"], reverse=True)[:top]:
    rmeta = u.get("rmeta_time")
    split = f"  (front {rmeta:.1f}s, codegen+link {u['duration'] - rmeta:.1f}s)" if rmeta else ""
    target = u.get("target", "").strip() or "lib"
    print(f"{u['duration']:7.1f}s  {u['name']} {target} {u.get('mode', '')}{split}")
print(report)
EOF
