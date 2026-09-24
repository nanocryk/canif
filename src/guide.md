# Writing canif scripts

Scripts live in `.canif/` directories: every `.canif/` from the cwd up, plus `~/.canif`. Nearest wins on name clashes.
Name = path under `.canif/` without extension: `.canif/cargo/test.sh` is `cargo/test`.
Entries starting with `.` or `_` are not scripts: put shared helpers in `_lib/`.
Interpreters: `.sh` bash (Git Bash on Windows), `.ps1` pwsh, `.py` python3 (`py` launcher on Windows).
Symlinked directories inside `.canif/` are not followed; symlinked files are scripts.

## Header

YAML in a comment block fenced by `---`, on the first line or right after the shebang. `#` or `//` comments.

```
#!/usr/bin/env bash
# ---
# description: Run tests of one crate
# args:
#   crate:
#     choices: [core, cli]
#   filter:
#     pattern: '[A-Za-z0-9_:]+'
#     optional: true
#     help: Test name filter
#   jobs:
#     type: int
#     min: 1
#     max: 16
#     optional: true
#     default: 4
#   release:
#     type: flag
# ---
set -euo pipefail
cargo test -p "$CANIF_CRATE" ${CANIF_RELEASE:+--release} -j "$CANIF_JOBS" ${CANIF_FILTER:+"$CANIF_FILTER"}
```

Called as: `canif run cargo/test core --filter foo --release`

- `description` is required. It is what `canif list <terms>` searches.
- `description` and `help` are folded to one line (whitespace runs become one space), so `|` / `>` blocks can wrap long text.
- Quote YAML values containing `: ` or starting with a special character: `'...'`.
- Arg names match `[a-z][a-z0-9-]*`. Key order is the positional order.
- `confirm: true`: the script only runs through `canif confirm <name> ...` (same args as `run`), which is not allowlisted, so every call prompts. `canif run` refuses it. Use for risky scripts (deploys, deletions).

## Arg kinds (at most one per arg)

- `choices: [a, b]`: one of the values.
- `pattern: <regex>`: must match the whole value (anchored automatically).
- `type: int`, optional `min` / `max`: integer in range.
- `type: flag`: bare `--name`, no value. Always optional.
- none: any text.

## Arg options

- `optional: true`: passed as `--name value` or `--name=value`.
- `named: true`: required but passed by name.
- `many: true`: takes all remaining positional values. Only one, and it must be the last positional.
- `help: ...`: shown by `canif show`.
- `default: <value>`: exported when an optional arg is omitted, shown in the usage line (`[--jobs <jobs>=4]`). Not allowed on required, `many` or flag args. Validated like a call-time value; an invalid default is a header error.

Required args without `named` are positional.

## Values

Rejected everywhere: leading `-` (except ints), newline, carriage return, NUL. No `--` pass-through: every value the script receives was validated.
Ints reach the script in canonical form: `+5` is `5`, `0008` is `8`.

## Script environment

- Runs in the caller's cwd; `cd "$CANIF_PROJECT"` if the script needs the project root.
- `CANIF_<NAME>` per arg, `-` becomes `_` (`dry-run` is `CANIF_DRY_RUN`). Absent optional args are unset, or set to their `default`. Flags are `1` or unset.
- The `many` arg is the script's argv (`"$@"`) and its env var joined with newlines.
- `CANIF_DIR`: the `.canif/` holding the script (`source "$CANIF_DIR/_lib/x.sh"`). `CANIF_PROJECT`: its parent.
- Inherited `CANIF_*` variables are cleared. The exit code is the script's.

## Config: `.canif/.config.yml`

```
root: true          # stop looking for .canif/ in parent directories
interpreters:
  sh: C:/Program Files/Git/bin/bash.exe
  js: node          # makes .js files scripts
```

Merged per key, nearer directory wins. An interpreter containing `/` or `\` is a path; relative ones resolve against the `.canif/` holding that config. Bare names go through `PATH`.

## After writing

Run `canif check`, then `canif show <name>` to confirm the usage line.
