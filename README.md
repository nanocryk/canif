# canif

Run reviewed scripts with validated arguments, so AI agents can be allowlisted on one command instead of prompting on every long shell line.

Scripts live in `.canif/` directories. Each declares its arguments in a header. The agent calls `canif run <script> <args>`; canif validates every value against the header before the script sees it. Review happens once, on the script, in git.

## Install

```sh
cargo install --path .
```

## Scripts

`.canif/cargo/test.sh` is the script `cargo/test`:

```sh
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
#   release:
#     type: flag
# ---
set -euo pipefail
cargo test -p "$CANIF_CRATE" ${CANIF_RELEASE:+--release} ${CANIF_FILTER:+"$CANIF_FILTER"}
```

```sh
canif run cargo/test core --filter foo --release
```

- Header: YAML in a comment block fenced by `---`, first line of the file or right after the shebang. `#` and `//` comments work. Quote values containing `: ` or starting with a YAML special character (`'...'`).
- Discovery: every `.canif/` from the cwd up, plus `~/.canif`. Nearest wins on name clashes. `root: true` in `.canif/.config.yml` stops the walk.
- Names: path under `.canif/` without extension. Entries starting with `.` or `_` are not scripts (`_lib/` for shared helpers).
- Interpreters: `.sh` bash (Git Bash on Windows), `.ps1` pwsh, `.py` python. Override or add in `.canif/.config.yml`:

  ```yaml
  interpreters:
    sh: C:/Program Files/Git/bin/bash.exe
    js: node
  ```

### Arguments

| Header | Accepts |
|---|---|
| `choices: [a, b]` | one of the listed values |
| `pattern: <regex>` | values matching the whole regex |
| `type: int` (`min`, `max`) | integers in range |
| `type: flag` | bare `--name`, no value |
| nothing | any text |

Options: `optional: true`, `named: true`, `many: true`, `help: ...`.

- Required args are positional, optional ones named: `--name value` or `--name=value`. `named: true` forces a required arg to be named.
- One `many` arg at most, the last positional; it takes the remaining values (shell globs expand into it).
- Every value is rejected if it starts with `-` (except ints), or contains a newline or NUL. Arg names match `[a-z][a-z0-9-]*`.

### Script environment

- Runs in the caller's cwd.
- `CANIF_<NAME>` per arg (`dry-run` is `CANIF_DRY_RUN`), flags are `1` or unset. The `many` arg is also the script's argv (`"$@"`), and newline-joined in its env var.
- `CANIF_DIR`: the `.canif/` holding the script. `CANIF_PROJECT`: its parent.
- Inherited `CANIF_*` variables are cleared first. Exit code is the script's.

## Commands

```
canif list [terms...]   scripts with usage lines, filtered by name/description
canif show <name>       full arg rules and source path
canif run <name> ...    validate, then run
canif check             validate every header, report conflicts and stray files
```

## Example scripts

`examples/global/` is a ready-made `~/.canif`: cargo scripts that print compact summaries instead of raw output, so agents stop wrapping cargo in `grep`/`awk` pipelines.

```
cargo/check [--package <package>] [--full]    one line per diagnostic, then counts
cargo/clippy [--package <package>] [--full]   same, warnings denied
cargo/fmt [--check]                           format, or list unformatted files
cargo/test [--package <package>] [--filter <filter>] [--nocapture] [--full]
                                              failures with panic messages, summed totals
cargo/verify [--package <package>]            fmt check, clippy, tests; stops at the first failure
```

Install with `cp -r examples/global/. ~/.canif/`. Shared helpers live in `_lib/cargo.sh`.

## Claude Code setup

Add to `~/.claude/settings.json` (or a project's `.claude/settings.json`):

```json
{
  "permissions": {
    "allow": [
      "Bash(canif run *)",
      "Bash(canif list*)",
      "Bash(canif show *)"
    ],
    "ask": [
      "Edit(**/.canif/**)",
      "Write(**/.canif/**)"
    ]
  }
}
```

Ask rules win over allow rules and over `acceptEdits` mode, so script changes always go through you. Scripts stay tracked in git: review them like code.

Install the agent skill by copying `skill/` to `~/.claude/skills/canif/`.
