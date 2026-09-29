---
name: canif
description: Use canif scripts instead of ad-hoc shell commands. TRIGGER before writing any build, test, lint, format, deploy, or other multi-flag shell command in a project; also when the user mentions canif or `.canif/`.
---

`canif auto` is allowlisted: it runs reviewed scripts without a permission prompt. Ad-hoc commands prompt the user.

- Before an ad-hoc command, run `canif list <terms>` (terms match script names and descriptions, any term). Plain `canif list` to see everything.
- Each line is a usage line: `<name> <required> [--optional <value>] [--flag]  # description`. Choices show inline as `<arg:a|b>`. `canif show <name>` gives patterns, int ranges and help.
- Call `canif auto <name> <args>` exactly as the usage line says. Required args positional, optional ones `--name value`. A trailing `<arg>...` takes several values; shell globs are fine there.
- A description starting with `[risky]` marks a script `canif auto` refuses: call it with `canif run <name> <args>`, which prompts the user every time.
- A validation error prints the broken rule and the usage line: fix the call and retry once. Values may not start with `-` (except ints) or contain newlines.
- Never edit `.canif/` directly to make a call pass. When a script is missing or too narrow, propose a new or changed script to the user (`canif guide` prints the script format), then use it once added.
- Trimmed output contains a `canif: output trimmed (...), full log: <path>` line: Read that file for the missing lines instead of re-running. `canif auto --head <n> --tail <n> <name> ...` (options before the name) changes the trimming for one call.
- `canif check` validates all headers after script changes.
