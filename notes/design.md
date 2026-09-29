# canif design

## Purpose

Agents (Claude Code) keep producing long, similar-looking shell commands that can't be allowlisted with a glob because some argument variants are risky. Reviewing each one means skimming flags nobody knows by heart.

canif moves the review from each command to each script: scripts are reviewed once, the agent only varies arguments, and canif validates those against types declared in the script. `canif auto *` is allowlisted in Claude Code; `canif auto` refuses scripts marked risky.

Aimed at AI agents, not humans: no interactive UI. Output is optimized to save agent round trips and tokens. Inspired by iforgor, minus the TUI.

## Scripts

- Plain script files under a `.canif/` directory, runnable and lintable on their own.
- Name = path relative to `.canif/` without extension: `.canif/cargo/test.sh` is `cargo/test`.
- Files and directories starting with `.` or `_` are never scripts. `_lib/` holds helpers scripts can source.
- Symlinked directories inside `.canif/` are not followed (no loops, no scanning outside the tree). Symlinked files are scripts like any other.
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
- An interpreter value containing `/` or `\` is a path: relative ones resolve against the `.canif/` directory holding that config. Bare names (`node`) go through `PATH`.

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
#     pattern: '[A-Za-z0-9_:]+'
#     optional: true
#     help: Test name filter
#   release:
#     type: flag
# ---
set -euo pipefail
cargo test -p "$CANIF_CRATE" ${CANIF_RELEASE:+--release} ${CANIF_FILTER:+"$CANIF_FILTER"}
```

- `description` is the only searchable text; no tags.
- `description` and `help` are folded to one line: every run of whitespace or control chars (newlines, tabs, NUL) becomes one space, ends trimmed. YAML block scalars (`|`, `>`) can wrap long text.
- `risky: true`: `canif auto` refuses the script, so it runs only through `canif run`, which stays off the allowlist so Claude Code prompts for every call. canif itself never prompts.
- `output: {head: <n>, tail: <n>}`: default trimming of the script's output (either key or both).
- `args` is a map; key order is the positional order.
- Arg names match `[a-z][a-z0-9-]*`. `dry-run` is passed as `--dry-run` and exported as `CANIF_DRY_RUN`. The charset makes this mapping collision-free.
- Arg kind is inferred:
  - `choices: [...]`: enum. A choice that call-time validation would reject (leading `-`, newline, NUL) is a header error.
  - `pattern: <regex>`: string matching the anchored regex.
  - `type: int`, with optional `min` / `max`.
  - `type: flag`: always optional and named (`--release`).
  - none of the above: free text.
- Arg options: `optional`, `help`, `named: true` (force named form on a required arg), `many: true`, `default`.
- `default`: optional non-flag args only. Validated against the arg's kind like a call-time value. Exported when the arg is omitted. Shown in the usage line: `[--count <count>=1]`.
- No `path` kind: paths are plain strings, validated with `pattern` when needed.

## Arguments at call time

- Required args are positional, optional args are named `--name value` or `--name=value` (clap convention).
- At most one `many` arg per script, and only as the last positional: `canif auto fmt/files a.rs b.rs`.
- Rejected in every value: leading `-` (except `int` values, so `-5` is valid), newline, NUL.
- No unvalidated rest args.

## Passing values to the script

- Values never reach the script source.
- Scalar args: env var `CANIF_<NAME>` only. Flags are `1` or unset.
- `int` values are passed in canonical decimal form: `+5` becomes `5`, `0008` becomes `8`.
- The `many` arg: script argv (`"$@"` in bash, `$args` in PowerShell), and env var `CANIF_<NAME>` newline-joined.

## Globs

- canif never expands globs.
- The calling shell (Claude Code's Bash tool is Git Bash, even on Windows) expands unquoted globs before canif runs, so they land in the `many` last positional. Commands are designed around that.
- An unquoted glob does not trigger a Claude Code permission prompt under a `Bash(canif <subcommand> *)` allow rule (observed with `glob-probe src/*.rs`).
- An unmatched glob reaches canif literally and goes through normal validation.
- Scripts that need a file set compute it themselves (e.g. from `git diff`) rather than taking a pattern.

## Execution

- Script runs in the caller's cwd.
- `CANIF_DIR`: the `.canif/` directory holding the running script (never one found from the cwd). Used to call other scripts and source libs: `source "$CANIF_DIR/_lib/common.sh"`.
- `CANIF_PROJECT`: parent of `CANIF_DIR`. Used for project paths: `cd "$CANIF_PROJECT"`.
- A script may call `canif auto other/script ...` to get validation on the nested call.
- Every inherited `CANIF_*` variable (case-insensitive) is removed before the script's own are set, so nested calls don't leak optional args.
- On Windows, `CANIF_DIR`, `CANIF_PROJECT` and the script path given to the interpreter use forward slashes.
- Interpreter invocation, program from config override or default:
  - `.sh`: bash. On Windows, Git for Windows bash, never a bare `bash`.
  - `.ps1`: `pwsh -NoProfile -NonInteractive -File <script>` (flags kept with an override).
  - `.py`: `py` (the Python launcher) on Windows, `python3` elsewhere.
  - config-added extension: `<program> <script> <args>`.
- stdout/stderr pass through, trimmed per [Output](#output). canif exits with the script's exit code (1 if it has none, e.g. killed by a signal).

## Output

- Trimming: `head` and `tail` from the header, overridden per call with options before the script name: `canif auto --tail 80 cargo/test core`.
- The full output of every run is written to `~/.canif/.logs/`. canif keeps the last 50 logs and deletes older ones on each run.
- When output was trimmed, canif prints the log path so the agent can read the rest instead of re-running. Untrimmed runs print no path.
- stdout and stderr are piped and read line by line (split on `\n`, bytes, a final unterminated line kept). Lines of both streams are merged in arrival order: that sequence is what `head` / `tail` count and what the log holds. Each printed line goes to the stream it came from.
- canif returns when the script exits, not when its pipes close: a background process started by the script can hold them open. After exit, output is still relayed until no line arrives for 100ms.
- On Windows the caller still waits for background processes a script leaves running: they inherit canif's own output handles.
- `head` lines print live, `tail` lines at exit. Output that fits in `head` + `tail` prints whole.
- Trimmed notice, on stderr where the gap is (after the head, before the tail): `canif: output trimmed (<dropped> of <total> lines), full log: <path>`, or `no log available` when there is no home or the log failed.
- Log file: `<unix-millis, 20 digits>-<pid>-<name, / as _>.log`, merged lines only. Retention keeps the 50 names that sort last. A log write or prune failure prints one `canif: ` warning and never changes the exit code.
- Header `output` values must be positive integers; 0, negatives and unknown keys are header errors. `show` prints `output: head <n>, tail <n>` (keys set only) after `source:` / `risky:`.

## Miss log

- `~/.canif/.misses.log`, shared by every project. One line per unknown script name or validation failure: timestamp, cwd, full argv, reason (unknown name, or the failing arg and rule).
- Capped at the last 1000 lines.
- Shows which scripts agents lack and which constraints are too tight.
- Written by `auto` and `run` only. `auto` refusing a risky script, header errors and script failures are not misses.
- Line format: tab-separated `<timestamp>\t<cwd>\t<name>\t<reason>\t<arg>...`, one field per argv value after the name (none when there are no args). `\`, tab, CR and LF inside a field are escaped as `\\`, `\t`, `\r`, `\n`. Timestamp: UTC `2026-09-24T13:05:09Z`. Reason: `unknown script`, or the validation error's first line (`arg `mode`: `nope` is not one of fast|slow`).
- Cap: append, then when the file holds more than 1000 lines, write the last 1000 to a temp file in the same directory and rename it over the log. Concurrent runs are best effort.
- A write failure prints one `canif: ` warning; output and exit code are unchanged. No home: nothing is written.
- `canif misses` output: one `<total>  <name>` line per script name, most misses first (ties by name), each followed by `     <count>  <reason>` lines, same order. argv, cwd and timestamps are not printed. Empty or missing log: `no misses recorded`. Malformed lines are skipped.

## CLI

- `canif list [terms...]`: scripts matching any term (case-insensitive substring) in name or description; all scripts without terms. Plain text, no JSON.
  - One `<.canif path>:` header line per origin directory that has listed scripts.
  - One line per script: `  <usage line>  # <description>`, e.g. `  cargo/test <crate:core|cli> [--filter <filter>] [--release]  # Run tests of one crate`.
  - A script with an invalid header is listed as `  <name>  # invalid header, run canif check`.
  - A `risky: true` script is marked so the agent picks the right subcommand on the first call: `  deploy/prod <env:staging|prod>  # [risky] Deploy the app`.
- `canif show <name>`: usage line (prefixed `canif run` for risky scripts, `canif auto` otherwise, as in validation errors), description, source path, `risky: true` for risky scripts, then one line per arg: `arg <name>: <kind>, required|optional[, many][, default: ...][, help: ...]`.
- `canif run [--head <n>] [--tail <n>] <name> [args...]`: validate, then exec, for any script. Everything after the name goes to the script's arg parsing, including `--help` and `--`. The command for humans, and for risky scripts.
- `canif auto [--head <n>] [--tail <n>] <name> [args...]`: same as `run`, but refuses `risky: true` scripts and names `canif run`. The command for allowlisting.
- `canif misses`: the miss log grouped by script name, with counts.
- `canif check`: validate all headers. Header errors, name conflicts, unreadable entries and invalid configs exit 1; unknown files are warnings. It reports every problem instead of stopping at the first; other commands fail on the first discovery problem.
- `canif guide`: script authoring reference (header format, arg kinds, env, config), compiled into the binary from `src/guide.md` so it is available wherever canif is installed. Its example is parsed by a unit test.

Errors:
- canif's own errors go to stderr prefixed `canif: `, exit code 2. Errors parsing canif's own arguments keep clap's format.
- Unknown script name: suggest the closest match, within one edit per 3 chars of the typed name (at least 1, at most 3), else the only name containing it.
- A closed stdout (`canif list | head`) is not an error for `list`, `show`, `misses`, `check`, `guide`.
- Validation failure: print the failing arg, the rule it broke, and the script's usage line, so the agent fixes it in one retry.

## Claude Code integration

- README documents a settings snippet; canif never edits Claude Code settings itself.
  - allow: `Bash(canif auto *)`, `Bash(canif list*)`, `Bash(canif show *)`, `Bash(canif check)`, `Bash(canif guide)`, `Bash(canif misses)`
  - `canif run` is deliberately left out, so risky scripts always prompt. No subcommand may start with `auto`: an allow rule is a plain prefix match, so `Bash(canif auto *)` would also approve `canif auto-<anything>` (observed with `Bash(canif run *)` approving `canif run-confirmed`).
  - ask: `Edit(**/.canif/**)`, `Edit(~/.canif/**)`. `Edit` rules cover every built-in file-editing tool; `**/` only matches under the current project.
- Ask rules take precedence over allow rules and over `acceptEdits` mode; only `bypassPermissions` skips them.
- Agents may read scripts and propose changes; writes go through user approval.
- A small agent-facing skill lives in the repo: run `canif list <terms>` before writing an ad-hoc command, plain `canif list` when exploring.

## Stack

Rust: `clap`, `serde`, `serde-saphyr`, `regex`.
