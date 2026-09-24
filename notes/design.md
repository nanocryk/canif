# canif design

## Purpose

Agents (Claude Code) keep producing long, similar-looking shell commands that can't be allowlisted with a glob because some argument variants are risky. Reviewing each one means skimming flags nobody knows by heart.

canif moves the review from each command to each script: scripts are reviewed once, the agent only varies arguments, and canif validates those against types declared in the script. `canif run *` is allowlisted in Claude Code.

Aimed at AI agents, not humans: no interactive UI. Output is optimized to save agent round trips and tokens. Inspired by iforgor, minus the TUI.

## Scripts

- Plain script files under a `.canif/` directory, runnable and lintable on their own.
- Name = path relative to `.canif/` without extension: `.canif/cargo/test.sh` is `cargo/test`.
- Files and directories starting with `.` or `_` are never scripts. `_lib/` holds helpers scripts can source.
- Interpreter chosen from the file extension: `.sh` bash, `.ps1` pwsh, `.py` python. Overridable per extension in config.
- On Windows, `.sh` runs with Git for Windows bash (derived from `git --exec-path`, then standard install paths), never `System32\bash.exe` (WSL).
- Files with an extension that has no interpreter are skipped by `list` and `run`, and reported by `check`.
- Project scripts are tracked in git. Git history and commit review are the integrity check: no trust store, no hash pinning.

## Discovery

- Walk up from the caller's cwd collecting every `.canif/` directory, plus the user's `~/.canif` (`%USERPROFILE%\.canif` on Windows).
- If the walk reaches home, `~/.canif` is counted once.
- Name clashes: nearest directory wins.
- Same name twice inside one `.canif/` (e.g. `build.sh` and `build.ps1`): conflict, reported by `check`. The name is unusable, and it still hides the same name in farther directories.
- `root: true` in a directory's config stops the walk there. `~/.canif` is still included.

## Config

- File: `.canif/.config.yml`. Dot prefix guarantees it never clashes with a script.
- Merged per key across all discovered directories; nearer overrides farther.
- Keys: `root: true`, interpreter path per extension (e.g. an explicit Git Bash path on Windows).

## Header

YAML inside a comment block fenced by `---` lines. Comment prefix `#` or `//` is stripped before parsing. Parsed with `serde-saphyr`.

- The opening fence must be the first line, or the second after a shebang. A UTF-8 BOM is ignored.
- A script without a header is an error: description and args are what agents rely on.

```sh
#!/usr/bin/env bash
# ---
# description: Run tests of one crate
# args:
#   crate:
#     choices: [core, cli]
#   filter:
#     pattern: ^[A-Za-z0-9_:]+$
#     optional: true
#     help: Test name filter
#   release:
#     type: flag
# ---
cargo test -p "$CANIF_CRATE" ${CANIF_RELEASE:+--release} -- "${CANIF_FILTER:-}"
```

- `description` is the only searchable text; no tags.
- `args` is a map; key order is the positional order.
- Arg names match `[a-z][a-z0-9-]*`. `dry-run` is passed as `--dry-run` and exported as `CANIF_DRY_RUN`. Names mapping to the same env var are rejected.
- Arg kind is inferred:
  - `choices: [...]`: enum. A choice that call-time validation would reject (leading `-`, newline, NUL) is a header error.
  - `pattern: <regex>`: string matching the anchored regex.
  - `type: int`, with optional `min` / `max`.
  - `type: flag`: always optional and named (`--release`).
  - none of the above: free text.
- Arg options: `optional`, `help`, `named: true` (force named form on a required arg), `many: true`.
- No `path` kind: paths are plain strings, validated with `pattern` when needed.

## Arguments at call time

- Required args are positional, optional args are named `--name value` or `--name=value` (clap convention).
- At most one `many` arg per script, and only as the last positional: `canif run fmt/files a.rs b.rs`.
- Rejected in every value: leading `-` (except `int` values, so `-5` is valid), newline, NUL.
- No unvalidated rest args.

## Passing values to the script

- Values never reach the script source.
- Scalar args: env var `CANIF_<NAME>` only. Flags are `1` or unset.
- The `many` arg: script argv (`"$@"` in bash, `$args` in PowerShell), and env var `CANIF_<NAME>` newline-joined.

## Globs

- canif never expands globs.
- The calling shell (Claude Code's Bash tool is Git Bash, even on Windows) expands unquoted globs before canif runs, so they land in the `many` last positional. Commands are designed around that.
- An unmatched glob reaches canif literally and goes through normal validation.
- Scripts that need a file set compute it themselves (e.g. from `git diff`) rather than taking a pattern.

## Execution

- Script runs in the caller's cwd.
- `CANIF_DIR`: the `.canif/` directory holding the running script (never one found from the cwd). Used to call other scripts and source libs: `source "$CANIF_DIR/_lib/common.sh"`.
- `CANIF_PROJECT`: parent of `CANIF_DIR`. Used for project paths: `cd "$CANIF_PROJECT"`.
- A script may call `canif run other/script ...` to get validation on the nested call.
- stdout/stderr pass through; exit code is preserved.

## CLI

- `canif list [terms...]`: scripts matching any term in name or description; all scripts without terms. Plain text, no JSON.
  - Grouped under one header line per origin directory.
  - One line per script: usage line with choices inline, then description. `cargo/test <crate:core|cli> [--filter <filter>] [--release]  Run tests of one crate`
- `canif show <name>`: full arg spec (patterns, ranges, help) and source path.
- `canif run <name> [args...]`: validate, then exec.
- `canif check`: validate all headers.

Errors:
- Unknown script name: suggest the closest match.
- Validation failure: print the failing arg, the rule it broke, and the script's usage line, so the agent fixes it in one retry.

## Claude Code integration

- README documents a settings snippet; canif never edits Claude Code settings itself.
  - allow: `Bash(canif run *)`, `Bash(canif list*)`, `Bash(canif show *)`
  - ask: `Edit(**/.canif/**)`, `Write(**/.canif/**)`
- Ask rules take precedence over allow rules and over `acceptEdits` mode; only `bypassPermissions` skips them.
- Agents may read scripts and propose changes; writes go through user approval.
- A small agent-facing skill lives in the repo: run `canif list <terms>` before writing an ad-hoc command, plain `canif list` when exploring.

## Stack

Rust: `clap`, `serde`, `serde-saphyr`, `regex`.

## Open

- Whether Claude Code prompts on an unquoted glob in `canif run ...` under the allow rule: test once the binary exists.
