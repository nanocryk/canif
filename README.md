# canif

Run reviewed scripts with validated arguments, so AI agents can be allowlisted on one command instead of prompting on every long shell line.

## How it works

Agents keep writing long, slightly different shell commands. A glob allowlist can't separate the safe variants from the risky ones, so every call prompts and you end up skimming flags.

canif moves the review from each command to each script. Scripts live in `.canif/` directories and declare their arguments in a header. The agent calls `canif run <script> <args>`, and canif validates every value against the header before the script sees it. You review the script once, in git; the agent only varies arguments.

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
canif run cargo/test core --filter foo --release   # runs
canif run cargo/test core --filter 'foo; rm -rf ~' # rejected, prints the rule and usage line
```

## Quick start

Install the binary:

```sh
cargo install canif
# or the latest from git
cargo install --git https://github.com/nanocryk/canif
```

Allow canif in Claude Code, in `~/.claude/settings.json` (or a project's `.claude/settings.json`):

```json
{
  "permissions": {
    "allow": [
      "Bash(canif run *)",
      "Bash(canif list*)",
      "Bash(canif show *)",
      "Bash(canif check)",
      "Bash(canif guide)",
      "Bash(canif misses)"
    ],
    "ask": [
      "Edit(**/.canif/**)",
      "Edit(~/.canif/**)"
    ]
  }
}
```

- `canif confirm` is left out of `allow` on purpose, so `confirm: true` scripts always prompt.
- The `ask` rules make every script change go through you. They win over allow rules and over `acceptEdits` mode. `Edit` rules cover every built-in file-editing tool (Write included). `**/.canif/**` only matches under the current project, hence the separate `~/.canif` rule.

Install the agent skill, which tells the agent to look for a script before writing an ad-hoc command: copy `skill/` to `~/.claude/skills/canif/`.

Optionally, start from the example scripts: `cp -r examples/global/. ~/.canif/` (see [Example scripts](#example-scripts)).

Then write your own scripts: `canif guide` prints the authoring reference.

## Commands

```
canif list [terms...]   scripts with usage lines, filtered by name/description
canif show <name>       full arg rules and source path
canif run [--head <n>] [--tail <n>] <name> ...
                        validate, then run
canif confirm [--head <n>] [--tail <n>] <name> ...
                        same, for `confirm: true` scripts only
canif misses            unknown names and validation failures, grouped by script name
canif check             validate every header and config, report conflicts and stray files
canif guide             script authoring reference (header format, arg kinds, env, config)
```

## Trust model

canif validates arguments; it does not sandbox scripts. An allowlisted `canif run` executes any script discovery finds, with your permissions.

- Discovery trusts every `.canif/` from the cwd up. In a cloned repo, its scripts run without a prompt and shadow same-named `~/.canif` scripts. Its `.canif/.config.yml` can also replace interpreters for every script, home ones included.
- Review a repo's `.canif/` before running canif in it, and again after pulls. `root: true` in `.canif/.config.yml` stops the walk upward. Keep `~/.canif` script names unlikely to clash.
- The `ask` rules above cover Claude Code's file-editing tools only. Any other allowlisted command that writes files (`git checkout`, `cp`, a script that writes files) can still change `.canif/`.
- Keep scripts tracked in git and review them like code.

## Writing scripts

`canif guide` is the full reference. It ships inside the binary, so agents can read it anywhere canif is installed. In short:

- **Location**: every `.canif/` from the cwd up, plus `~/.canif`. Nearest wins on name clashes.
- **Names**: path under `.canif/` without extension. Entries starting with `.` or `_` are not scripts (`_lib/` for shared helpers).
- **Interpreters**: `.sh` bash (Git Bash on Windows), `.ps1` pwsh, `.py` python3 (`py` launcher on Windows). Override or add extensions in `.canif/.config.yml`.
- **Header**: YAML in a comment block (`#` or `//`) fenced by `---`, on the first line or right after the shebang. `description` is required.

| Arg header | Accepts |
|---|---|
| `choices: [a, b]` | one of the listed values |
| `pattern: <regex>` | values matching the whole regex |
| `type: int` (`min`, `max`) | integers in range |
| `type: flag` | bare `--name`, no value |
| nothing | any text |

- **Options**: `optional: true`, `named: true`, `many: true`, `default: <value>`, `help: ...`. Required args are positional, optional ones named (`--name value` or `--name=value`). One `many` arg at most, the last positional; shell globs expand into it.
- **Values**: rejected if they start with `-` (except ints), or contain a newline, carriage return or NUL.
- **Environment**: `CANIF_<NAME>` per arg (`dry-run` is `CANIF_DRY_RUN`), flags `1` or unset. `CANIF_DIR` is the `.canif/` holding the script, `CANIF_PROJECT` its parent. The script runs in the caller's cwd and its exit code is canif's.
- **`confirm: true`**: the script only runs through `canif confirm`, which is not allowlisted. Use it for deploys and deletions.
- **`output: {head: <n>, tail: <n>}`**: default trimming of the script's output, see [Output](#output).

Run `canif check` after writing a script.

## Output

- stdout and stderr are relayed line by line, each line to the stream it came from. Trimming counts lines over both streams merged in arrival order.
- `--head` / `--tail` go before the script name (`canif run --tail 80 cargo/test core`) and replace the header's value for that key. Everything after the name goes to the script.
- `head` lines print live; `tail` lines print when the script exits. When both are set and the output fits, everything prints once.
- Every run's full output is logged to `~/.canif/.logs/<unix-millis>-<pid>-<name>.log` (`/` in the name becomes `_`). The newest 50 logs are kept.
- When lines were dropped, canif prints `canif: output trimmed (<dropped> of <total> lines), full log: <path>` to stderr where the gap is, so the agent reads the log instead of re-running. A log that cannot be written prints one warning and never fails the run.

## Miss log

Every `canif run` / `canif confirm` call with an unknown script name or an invalid arg is appended to `~/.canif/.misses.log`, shared by all projects (last 1000 lines kept). `canif misses` groups it by script name, most misses first:

```
3  cargo/tets
     3  unknown script
2  cargo/test
     1  arg `count`: 9 is out of range, must be in 1..=5
     1  arg `crate`: `nope` is not one of core|cli
```

It shows which scripts agents look for and don't find, and which arg rules are too tight.

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

Shared helpers live in `_lib/cargo.sh`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
