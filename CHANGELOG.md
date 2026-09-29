# Changelog

## 0.1.0

Initial release.

- `canif run <name> [args...]`: validates args against the script's header, then runs it with values in `CANIF_<NAME>` env vars (and argv for the `many` arg).
- `canif auto <name> [args...]`: same as `run`, but refuses scripts with `risky: true`. Meant for the allowlist, with `run` kept off it so risky scripts always prompt.
- `canif list [terms...]`, `canif show <name>`, `canif check`, `canif guide`.
- Discovery of every `.canif/` from the cwd up plus `~/.canif`, nearest wins; `root: true` stops the walk.
- Arg kinds: `choices`, `pattern` (anchored regex), `type: int` with `min` / `max`, `type: flag`, free text. Options: `optional`, `named`, `many`, `default`, `help`.
- Interpreters by extension: `.sh` bash (Git Bash on Windows), `.ps1` pwsh, `.py` python3 (`py` on Windows); overridable and extensible in `.canif/.config.yml`.
- Output trimming: `output: {head, tail}` in the header, `--head` / `--tail` per call. Every run is logged to `~/.canif/.logs/`, and the log path is printed when lines were dropped.
- Miss log: unknown names and validation failures go to `~/.canif/.misses.log`; `canif misses` groups them by script name.
- Example cargo scripts with summarized output in `examples/global/`, and a Claude Code skill in `skill/`.
