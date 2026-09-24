# canif

Run reviewed scripts with validated arguments, so AI agents can be allowlisted on one command instead of prompting on every long shell line.

Scripts live in `.canif/` directories. Each declares its arguments in a header. The agent calls `canif run <script> <args>`; canif validates every value against the header before the script sees it. Review happens once, on the script, in git.

![canif development as an MS Paint comic](https://raw.githubusercontent.com/nanocryk/canif/main/comic.png)

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
- Interpreters: `.sh` bash (Git Bash on Windows), `.ps1` pwsh, `.py` python3 (`py` launcher on Windows). Override or add in `.canif/.config.yml`:

  ```yaml
  interpreters:
    sh: C:/Program Files/Git/bin/bash.exe
    js: node
  ```

  A value containing `/` or `\` is a path: relative ones resolve against the `.canif/` holding the config. Bare names go through `PATH`.
- Symlinked directories inside `.canif/` are not followed; symlinked files are scripts.
- `description` and `help` are folded to one line, so YAML `|` / `>` blocks can wrap long text.
- `confirm: true` in the header: the script runs only through `canif confirm <name> ...`, which is not allowlisted, so every call prompts. `canif run` refuses it and names the right command; `canif list` marks it `[confirm]`.

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
- One `many` arg at most, the last positional; it takes the remaining values (shell globs expand into it). Unquoted globs don't trigger a Claude Code prompt under the `Bash(canif run *)` rule.
- Every value is rejected if it starts with `-` (except ints), or contains a newline or NUL. Arg names match `[a-z][a-z0-9-]*`.

### Script environment

- Runs in the caller's cwd.
- `CANIF_<NAME>` per arg (`dry-run` is `CANIF_DRY_RUN`), flags are `1` or unset, ints in canonical form (`0008` is `8`). The `many` arg is also the script's argv (`"$@"`), and newline-joined in its env var.
- `CANIF_DIR`: the `.canif/` holding the script. `CANIF_PROJECT`: its parent.
- Inherited `CANIF_*` variables are cleared first. Exit code is the script's.

## Commands

```
canif list [terms...]   scripts with usage lines, filtered by name/description
canif show <name>       full arg rules and source path
canif run <name> ...    validate, then run
canif confirm <name> ...
                        same, for `confirm: true` scripts only
canif check             validate every header and config, report conflicts and stray files
canif guide             script authoring reference (header format, arg kinds, env, config)
```

`canif guide` is the full reference for writing scripts; it ships inside the binary, so agents can read it anywhere canif is installed.

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
      "Bash(canif show *)",
      "Bash(canif check)",
      "Bash(canif guide)"
    ],
    "ask": [
      "Edit(**/.canif/**)",
      "Edit(~/.canif/**)"
    ]
  }
}
```

`canif confirm` is deliberately left out of `allow`, so `confirm: true` scripts always prompt. `Edit` rules cover every built-in file-editing tool (Write included). `**/.canif/**` only matches under the current project, hence the separate `~/.canif` rule. Ask rules win over allow rules and over `acceptEdits` mode, so script changes always go through you. Scripts stay tracked in git: review them like code.

Install the agent skill by copying `skill/` to `~/.claude/skills/canif/`.

## Trust model

canif validates arguments; it does not sandbox scripts. An allowlisted `canif run` executes any script discovery finds, with your permissions.

- Discovery trusts every `.canif/` from the cwd up. In a cloned repo, its scripts run without a prompt and shadow same-named `~/.canif` scripts. Its `.canif/.config.yml` can also replace interpreters for every script, home ones included.
- Review a repo's `.canif/` before running canif in it, and again after pulls. `root: true` stops the walk upward. Keep `~/.canif` script names unlikely to clash.
- The `ask` rules above cover Claude Code's file-editing tools only. Any other allowlisted command that writes files (`git checkout`, `cp`, a script that writes files) can still change `.canif/`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
