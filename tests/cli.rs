use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ECHO_ALL: &str = "#!/usr/bin/env bash
# ---
# description: Echo everything
# args:
#   mode:
#     choices: [fast, slow]
#   files:
#     many: true
#   filter:
#     pattern: ^[a-z]+$
#     optional: true
#     help: Name filter
#   release:
#     type: flag
# ---
echo \"dir=$CANIF_DIR\"
echo \"project=$CANIF_PROJECT\"
echo \"mode=$CANIF_MODE\"
echo \"filter=${CANIF_FILTER-unset}\"
echo \"release=${CANIF_RELEASE-unset}\"
for file in \"$@\"; do echo \"argv=$file\"; done
echo \"files=$CANIF_FILES\"
exit 3
";

const CARGO_TEST: &str = "#!/usr/bin/env bash
# ---
# description: Run tests of one crate
# args:
#   crate:
#     choices: [core, cli]
#   count:
#     type: int
#     min: 1
#     max: 5
#     optional: true
#   release:
#     type: flag
# ---
echo test
";

const GREET: &str = "# ---
# description: Say hello from home
# ---
echo \"hello $CANIF_DIR\"
";

const BROKEN: &str = "#!/usr/bin/env bash
echo no header
";

struct Fixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        // The child reads its cwd back fully resolved (macOS resolves `/var` to `/private/var`).
        // Windows is skipped: canonicalize adds a `\\?\` prefix there.
        #[cfg(not(windows))]
        let base = fs::canonicalize(root.path()).unwrap();
        #[cfg(windows)]
        let base = root.path().to_path_buf();
        let home = base.join("home");
        let project = base.join("project");
        fs::create_dir_all(&home).unwrap();
        let fixture = Self {
            _root: root,
            home,
            project,
        };
        fixture.write(".canif/.config.yml", "root: true\n");
        fixture
    }

    fn write(&self, relative: &str, content: &str) {
        write(&self.project, relative, content);
    }

    fn write_home(&self, relative: &str, content: &str) {
        write(&self.home, relative, content);
        write(&self.home, ".canif/.config.yml", "root: true\n");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_canif"));
        command
            .args(args)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home);
        command
    }

    fn canif(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn canif_dir(&self) -> PathBuf {
        self.project.join(".canif")
    }
}

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn portable(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

fn listed_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write(".canif/echo/all.sh", ECHO_ALL);
    fixture.write(".canif/cargo/test.sh", CARGO_TEST);
    fixture.write(".canif/broken.sh", BROKEN);
    fixture.write_home(".canif/greet.sh", GREET);
    fixture
}

#[test]
fn list_groups_by_origin() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["list"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let expected = format!(
        "{}:
  broken  # invalid header, run canif check
  cargo/test <crate:core|cli> [--count <count>] [--release]  # Run tests of one crate
  echo/all <mode:fast|slow> <files>... [--filter <filter>] [--release]  # Echo everything
{}:
  greet  # Say hello from home
",
        fixture.canif_dir().display(),
        fixture.home.join(".canif").display(),
    );
    assert_eq!(stdout(&output), expected);
}

#[test]
fn list_filters_by_any_term_case_insensitively() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["list", "HELLO", "zzz"]);
    let expected = format!(
        "{}:\n  greet  # Say hello from home\n",
        fixture.home.join(".canif").display()
    );
    assert_eq!(stdout(&output), expected);

    let output = fixture.canif(&["list", "cargo", "echo"]);
    let text = stdout(&output);
    assert!(text.contains("  cargo/test "), "{text}");
    assert!(text.contains("  echo/all "), "{text}");
    assert!(!text.contains("greet"), "{text}");
    assert!(!text.contains("broken"), "{text}");

    let output = fixture.canif(&["list", "broken"]);
    assert!(stdout(&output).contains("  broken  # invalid header"));
}

#[test]
fn list_without_match() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["list", "nothing", "here"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "no script matches: nothing here\n");
}

#[test]
fn list_without_scripts() {
    let fixture = Fixture::new();
    let output = fixture.canif(&["list"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "no .canif scripts found\n");
}

#[test]
fn show_prints_arg_rules() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["show", "echo/all"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let expected = format!(
        "usage: canif auto echo/all <mode:fast|slow> <files>... [--filter <filter>] [--release]
description: Echo everything
source: {}
arg mode: choices fast|slow, required
arg files: text, required, many
arg filter: pattern ^[a-z]+$, optional, help: Name filter
arg release: flag, optional
",
        fixture.canif_dir().join("echo").join("all.sh").display()
    );
    assert_eq!(stdout(&output), expected);

    let output = fixture.canif(&["show", "cargo/test"]);
    assert!(stdout(&output).contains("arg count: int 1..=5, optional\n"));
}

#[test]
fn show_broken_header_fails() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["show", "broken"]);
    assert_eq!(output.status.code(), Some(2));
    let text = stderr(&output);
    assert!(text.starts_with("canif: "), "{text}");
    assert!(text.contains("broken.sh: missing header"), "{text}");
}

#[test]
fn run_passes_env_argv_and_exit_code() {
    let fixture = listed_fixture();
    let output = fixture.canif(&[
        "run",
        "echo/all",
        "fast",
        "a.rs",
        "b c.rs",
        "--filter",
        "foo",
        "--release",
    ]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let expected = format!(
        "dir={}
project={}
mode=fast
filter=foo
release=1
argv=a.rs
argv=b c.rs
files=a.rs
b c.rs
",
        portable(&fixture.canif_dir()),
        portable(&fixture.project),
    );
    assert_eq!(stdout(&output), expected);
}

#[test]
fn run_does_not_leak_inherited_canif_vars() {
    let fixture = listed_fixture();
    let output = fixture
        .command(&["run", "echo/all", "slow", "x"])
        .env("CANIF_FILTER", "leak")
        .env("CANIF_RELEASE", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("filter=unset\n"), "{text}");
    assert!(text.contains("release=unset\n"), "{text}");
}

#[test]
fn run_validation_failure() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["run", "echo/all", "nope", "a.rs"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output).replace("\r\n", "\n"),
        "canif: arg `mode`: `nope` is not one of fast|slow
usage: canif auto echo/all <mode:fast|slow> <files>... [--filter <filter>] [--release]
"
    );
}

#[test]
fn run_unknown_name_suggests() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["run", "echo/al", "fast"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output).trim_end(),
        "canif: unknown script `echo/al`, did you mean `echo/all`?"
    );

    let output = fixture.canif(&["run", "completely-unrelated"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("run `canif list`"));
}

#[test]
fn run_home_script() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["run", "greet"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("hello {}\n", portable(&fixture.home.join(".canif")))
    );
}

#[test]
fn check_success() {
    let fixture = Fixture::new();
    fixture.write(".canif/echo/all.sh", ECHO_ALL);
    fixture.write(".canif/cargo/test.sh", CARGO_TEST);
    let output = fixture.canif(&["check"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "2 scripts OK\n");
}

#[test]
fn check_failure() {
    let fixture = Fixture::new();
    fixture.write(".canif/ok.sh", GREET);
    fixture.write(".canif/broken.sh", BROKEN);
    fixture.write(".canif/build.sh", GREET);
    fixture.write(".canif/build.ps1", GREET);
    fixture.write(".canif/notes.txt", "notes");
    let output = fixture.canif(&["check"]);
    assert_eq!(output.status.code(), Some(1));
    let canif_dir = fixture.canif_dir();
    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines[0].starts_with(&format!(
            "{}: missing header",
            canif_dir.join("broken.sh").display()
        )),
        "{text}"
    );
    assert_eq!(
        lines[1],
        format!(
            "conflict: build: {}, {}",
            canif_dir.join("build.ps1").display(),
            canif_dir.join("build.sh").display()
        )
    );
    assert_eq!(
        lines[2],
        format!(
            "warning: not a script (unknown extension): {}",
            canif_dir.join("notes.txt").display()
        )
    );
    assert_eq!(lines[3], "2 problems");
}

#[test]
fn check_reports_invalid_config_and_keeps_going() {
    let fixture = Fixture::new();
    fixture.write(".canif/broken.sh", BROKEN);
    write(&fixture.home, ".canif/.config.yml", "bogus: 1\n");
    let output = fixture.canif(&["check"]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    let home_config = fixture.home.join(".canif").join(".config.yml");
    assert!(
        text.starts_with(&format!("{}: invalid config", home_config.display())),
        "{text}"
    );
    assert!(text.contains("broken.sh: missing header"), "{text}");
    assert!(text.ends_with("\n2 problems\n"), "{text}");

    let output = fixture.canif(&["list"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("invalid config"));
}

const WITH_DEFAULTS: &str = "#!/usr/bin/env bash
# ---
# description: Echo defaults
# args:
#   count:
#     type: int
#     min: 1
#     optional: true
#     default: '03'
#     help: Repeat count
#   label:
#     optional: true
#     default: hi
# ---
echo \"count=$CANIF_COUNT label=$CANIF_LABEL\"
";

#[test]
fn run_exports_defaults_and_given_values_win() {
    let fixture = Fixture::new();
    fixture.write(".canif/defaults.sh", WITH_DEFAULTS);
    let output = fixture.canif(&["run", "defaults"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "count=3 label=hi\n");

    let output = fixture.canif(&["run", "defaults", "--count", "7", "--label=yo"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "count=7 label=yo\n");
}

#[test]
fn show_and_list_print_defaults() {
    let fixture = Fixture::new();
    fixture.write(".canif/defaults.sh", WITH_DEFAULTS);
    let output = fixture.canif(&["show", "defaults"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let expected = format!(
        "usage: canif auto defaults [--count <count>=3] [--label <label>=hi]
description: Echo defaults
source: {}
arg count: int >= 1, optional, default: 3, help: Repeat count
arg label: text, optional, default: hi
",
        fixture.canif_dir().join("defaults.sh").display()
    );
    assert_eq!(stdout(&output), expected);

    let output = fixture.canif(&["list"]);
    assert!(
        stdout(&output)
            .contains("  defaults [--count <count>=3] [--label <label>=hi]  # Echo defaults\n")
    );
}

#[test]
fn check_reports_invalid_default() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/bad.sh",
        &WITH_DEFAULTS.replace("default: '03'", "default: 0"),
    );
    let output = fixture.canif(&["check"]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(
        text.contains("arg `count`: invalid default: 0 is out of range, must be >= 1"),
        "{text}"
    );

    let output = fixture.canif(&["run", "bad"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("invalid default"));
}

const DEPLOY: &str = "#!/usr/bin/env bash
# ---
# description: Deploy the app
# risky: true
# args:
#   env:
#     choices: [staging, prod]
# ---
echo \"deploying $CANIF_ENV\"
exit 4
";

fn risky_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write(".canif/deploy/prod.sh", DEPLOY);
    fixture.write(".canif/greet.sh", GREET);
    fixture
}

#[test]
fn auto_refuses_risky_script_before_validation() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["auto", "deploy/prod", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        stderr(&output).trim_end(),
        "canif: script `deploy/prod` has `risky: true`, run it with `canif run deploy/prod`"
    );
}

#[test]
fn auto_runs_routine_scripts() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["auto", "greet"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(!output.stdout.is_empty());
}

#[test]
fn run_runs_risky_script() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["run", "deploy/prod", "staging"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert_eq!(stdout(&output), "deploying staging\n");
}

#[test]
fn risky_validation_failure_shows_its_usage() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["run", "deploy/prod", "dev"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output).replace("\r\n", "\n"),
        "canif: arg `env`: `dev` is not one of staging|prod
usage: canif run deploy/prod <env:staging|prod>
"
    );
}

#[test]
fn auto_unknown_name_suggests() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["auto", "deploy/prd", "staging"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output).trim_end(),
        "canif: unknown script `deploy/prd`, did you mean `deploy/prod`?"
    );
}

#[test]
fn list_and_show_mark_risky_scripts() {
    let fixture = risky_fixture();
    let output = fixture.canif(&["list"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let expected = format!(
        "{}:
  deploy/prod <env:staging|prod>  # [risky] Deploy the app
  greet  # Say hello from home
",
        fixture.canif_dir().display()
    );
    assert_eq!(stdout(&output), expected);

    let output = fixture.canif(&["show", "deploy/prod"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let expected = format!(
        "usage: canif run deploy/prod <env:staging|prod>
description: Deploy the app
source: {}
risky: true
arg env: choices staging|prod, required
",
        fixture.canif_dir().join("deploy").join("prod.sh").display()
    );
    assert_eq!(stdout(&output), expected);

    let output = fixture.canif(&["show", "greet"]);
    assert!(!stdout(&output).contains("risky"));
}

const COUNT: &str = "#!/usr/bin/env bash
# ---
# description: Print numbered lines
# args:
#   lines:
#     type: int
# ---
for i in $(seq 1 \"$CANIF_LINES\"); do echo \"line $i\"; done
";

fn numbered(range: std::ops::RangeInclusive<usize>) -> String {
    range.map(|index| format!("line {index}\n")).collect()
}

fn logs_dir(fixture: &Fixture) -> PathBuf {
    fixture.home.join(".canif").join(".logs")
}

fn log_files(fixture: &Fixture) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(logs_dir(fixture))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

/// The log path from the trimmed notice, checked against the notice format.
fn trimmed_log_path(stderr_text: &str, dropped: usize, total: usize) -> PathBuf {
    let prefix = format!("canif: output trimmed ({dropped} of {total} lines), full log: ");
    let line = stderr_text
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no trimmed notice in {stderr_text:?}"));
    PathBuf::from(&line[prefix.len()..])
}

#[test]
fn untrimmed_output_is_unchanged_and_logged() {
    let fixture = Fixture::new();
    fixture.write(".canif/count.sh", COUNT);
    fixture.write(
        ".canif/partial.sh",
        "# ---\n# description: x\n# ---\nprintf 'a\\n\\xff\\nb'\n",
    );
    let output = fixture.canif(&["run", "count", "3"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), numbered(1..=3));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));

    let output = fixture.canif(&["run", "partial"]);
    assert_eq!(output.stdout, b"a\n\xff\nb");
    let logs = log_files(&fixture);
    assert_eq!(logs.len(), 2);
    let log_ending = |suffix: &str| {
        let path = logs
            .iter()
            .find(|path| path.to_string_lossy().ends_with(suffix))
            .unwrap_or_else(|| panic!("no {suffix} in {logs:?}"));
        fs::read(path).unwrap()
    };
    assert_eq!(log_ending("-count.log"), numbered(1..=3).as_bytes());
    assert_eq!(log_ending("-partial.log"), b"a\n\xff\nb");
}

#[test]
fn header_tail_trims_and_prints_the_log_path() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/sub/count.sh",
        &COUNT.replace("# args:", "# output: {tail: 3}\n# args:"),
    );
    let output = fixture.canif(&["run", "sub/count", "20"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), numbered(18..=20));
    let log = trimmed_log_path(&stderr(&output), 17, 20);
    assert_eq!(log.parent().unwrap(), logs_dir(&fixture));
    let name = log.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("-sub_count.log"), "{name}");
    assert_eq!(fs::read_to_string(&log).unwrap(), numbered(1..=20));

    let output = fixture.canif(&["run", "sub/count", "3"]);
    assert_eq!(stdout(&output), numbered(1..=3));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn cli_head_overrides_header_and_keeps_its_tail() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/count.sh",
        &COUNT.replace("# args:", "# output: {head: 1, tail: 2}\n# args:"),
    );
    let output = fixture.canif(&["run", "--head", "3", "count", "10"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let mut expected = numbered(1..=3);
    expected.push_str(&numbered(9..=10));
    assert_eq!(stdout(&output), expected);
    trimmed_log_path(&stderr(&output), 5, 10);

    let output = fixture.canif(&["run", "--head", "3", "--tail", "4", "count", "6"]);
    assert_eq!(stdout(&output), numbered(1..=6));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn auto_accepts_trim_options() {
    let fixture = Fixture::new();
    fixture.write(".canif/count.sh", COUNT);
    let output = fixture.canif(&["auto", "--tail", "2", "count", "5"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), numbered(4..=5));
}

#[test]
fn stderr_lines_stay_on_stderr() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/mixed.sh",
        "# ---\n# description: x\n# output: {head: 1, tail: 1}\n# ---\necho out1\necho err1 >&2\necho out2\necho err2 >&2\nexit 5\n",
    );
    let output = fixture.canif(&["run", "mixed"]);
    assert_eq!(output.status.code(), Some(5));
    let out = stdout(&output);
    let err = stderr(&output);
    assert!(!out.contains("err"), "{out}");
    assert!(out.lines().all(|line| line.starts_with("out")), "{out}");
    let log = trimmed_log_path(&err, 2, 4);
    let script_err: Vec<&str> = err
        .lines()
        .filter(|line| !line.starts_with("canif: "))
        .collect();
    assert!(
        script_err.iter().all(|line| line.starts_with("err")),
        "{err}"
    );
    let mut logged: Vec<String> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    logged.sort();
    assert_eq!(logged, ["err1", "err2", "out1", "out2"]);

    let output = fixture.canif(&["run", "--head", "4", "mixed"]);
    assert_eq!(stdout(&output), "out1\nout2\n");
    assert_eq!(stderr(&output), "err1\nerr2\n");
}

#[test]
fn run_keeps_the_newest_50_logs() {
    let fixture = Fixture::new();
    fixture.write(".canif/count.sh", COUNT);
    for index in 0..55 {
        write(
            &logs_dir(&fixture),
            &format!("{index:020}-1-old.log"),
            "old\n",
        );
    }
    write(&logs_dir(&fixture), "keep.txt", "not a log\n");
    let output = fixture.canif(&["run", "count", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    let logs = log_files(&fixture);
    assert_eq!(logs.len(), 51);
    assert!(logs.contains(&logs_dir(&fixture).join("keep.txt")));
    assert!(!logs.contains(&logs_dir(&fixture).join(format!("{:020}-1-old.log", 5))));
    assert!(logs.contains(&logs_dir(&fixture).join(format!("{:020}-1-old.log", 6))));
    let newest = logs
        .iter()
        .find(|path| path.to_string_lossy().ends_with("-count.log"))
        .unwrap();
    assert_eq!(fs::read_to_string(newest).unwrap(), "line 1\n");
}

#[test]
fn unwritable_log_warns_once_and_runs() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/count.sh",
        &COUNT.replace("# args:", "# output: {tail: 1}\n# args:"),
    );
    // A file where the logs directory should be makes every log write fail.
    write(&fixture.home, ".canif/.logs", "");
    let output = fixture.canif(&["run", "count", "3"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "line 3\n");
    let err = stderr(&output);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 2, "{err}");
    assert!(lines[0].starts_with("canif: cannot write log "), "{err}");
    assert_eq!(
        lines[1],
        "canif: output trimmed (2 of 3 lines), no log available"
    );
}

#[test]
fn show_prints_output_trimming() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/count.sh",
        &COUNT.replace("# args:", "# output: {tail: 3, head: 2}\n# args:"),
    );
    fixture.write(
        ".canif/tail.sh",
        &COUNT.replace("# args:", "# output: {tail: 3}\n# args:"),
    );
    let output = fixture.canif(&["show", "count"]);
    let expected = format!(
        "usage: canif auto count <lines>
description: Print numbered lines
source: {}
output: head 2, tail 3
arg lines: int, required
",
        fixture.canif_dir().join("count.sh").display()
    );
    assert_eq!(stdout(&output), expected);
    let output = fixture.canif(&["show", "tail"]);
    assert!(stdout(&output).contains("\noutput: tail 3\n"));
}

#[test]
fn invalid_output_is_a_header_error() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/zero.sh",
        &COUNT.replace("# args:", "# output: {head: 0}\n# args:"),
    );
    fixture.write(
        ".canif/negative.sh",
        &COUNT.replace("# args:", "# output: {tail: -2}\n# args:"),
    );
    fixture.write(
        ".canif/unknown.sh",
        &COUNT.replace("# args:", "# output: {lines: 2}\n# args:"),
    );
    let output = fixture.canif(&["check"]);
    assert_eq!(output.status.code(), Some(1));
    let text = stdout(&output);
    assert!(
        text.contains("`output.head` must be a positive integer, got 0"),
        "{text}"
    );
    assert!(
        text.contains("`output.tail` must be a positive integer, got -2"),
        "{text}"
    );
    assert!(text.ends_with("3 problems\n"), "{text}");

    let output = fixture.canif(&["run", "zero", "1"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[test]
fn background_process_does_not_hold_the_run() {
    let fixture = Fixture::new();
    fixture.write(
        ".canif/spawn.sh",
        "# ---\n# description: x\n# ---\nsleep 30 &\necho started\nexit 0\n",
    );
    // Files, not pipes: Windows passes canif's own inheritable handles on to `sleep`,
    // so a piped `output()` would wait for `sleep` whatever canif does.
    let stdout_path = fixture.home.join("stdout.txt");
    let stderr_path = fixture.home.join("stderr.txt");
    let started = std::time::Instant::now();
    let status = fixture
        .command(&["run", "spawn"])
        .stdout(fs::File::create(&stdout_path).unwrap())
        .stderr(fs::File::create(&stderr_path).unwrap())
        .status()
        .unwrap();
    let elapsed = started.elapsed();
    let stderr_text = fs::read_to_string(&stderr_path).unwrap();
    assert_eq!(status.code(), Some(0), "{stderr_text}");
    assert_eq!(fs::read_to_string(&stdout_path).unwrap(), "started\n");
    assert!(stderr_text.is_empty(), "{stderr_text}");
    assert!(elapsed.as_secs() < 10, "took {elapsed:?}");
}

fn misses_path(fixture: &Fixture) -> PathBuf {
    fixture.home.join(".canif").join(".misses.log")
}

fn miss_lines(fixture: &Fixture) -> Vec<String> {
    match fs::read_to_string(misses_path(fixture)) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(_) => Vec::new(),
    }
}

/// Checks the timestamp shape and cwd, returns the name, reason and argv fields.
fn miss_fields(fixture: &Fixture, line: &str) -> Vec<String> {
    let fields: Vec<&str> = line.split('\t').collect();
    let timestamp = fields[0];
    assert_eq!(timestamp.len(), 20, "{line}");
    assert!(
        timestamp.ends_with('Z') && &timestamp[10..11] == "T",
        "{line}"
    );
    assert_eq!(
        fields[1],
        fixture.project.display().to_string().replace('\\', "\\\\")
    );
    fields[2..]
        .iter()
        .map(|field| (*field).to_owned())
        .collect()
}

#[test]
fn unknown_name_and_validation_failure_are_logged() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["run", "echo/al", "fast", "a\tb"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output).trim_end(),
        "canif: unknown script `echo/al`, did you mean `echo/all`?"
    );
    let output = fixture.canif(&["run", "echo/all", "nope", "x y", "--release"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!stderr(&output).contains("miss"), "{}", stderr(&output));

    let lines = miss_lines(&fixture);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        miss_fields(&fixture, &lines[0]),
        ["echo/al", "unknown script", "fast", "a\\tb"]
    );
    assert_eq!(
        miss_fields(&fixture, &lines[1]),
        [
            "echo/all",
            "arg `mode`: `nope` is not one of fast|slow",
            "nope",
            "x y",
            "--release"
        ]
    );
}

#[test]
fn auto_misses_are_logged() {
    let fixture = risky_fixture();
    fixture.canif(&["auto", "deploy/prd"]);
    fixture.canif(&["run", "deploy/prod", "dev"]);
    let lines = miss_lines(&fixture);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        miss_fields(&fixture, &lines[0]),
        ["deploy/prd", "unknown script"]
    );
    assert_eq!(
        miss_fields(&fixture, &lines[1]),
        [
            "deploy/prod",
            "arg `env`: `dev` is not one of staging|prod",
            "dev"
        ]
    );
}

#[test]
fn successes_and_refusals_are_not_misses() {
    let fixture = risky_fixture();
    fixture.write(".canif/broken.sh", BROKEN);
    let output = fixture.canif(&["run", "greet"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let output = fixture.canif(&["auto", "deploy/prod", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    let output = fixture.canif(&["run", "broken"]);
    assert_eq!(output.status.code(), Some(2));
    let output = fixture.canif(&["show", "nothing"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!misses_path(&fixture).exists());
}

#[test]
fn miss_log_keeps_the_last_1000_lines() {
    let fixture = listed_fixture();
    let prefill: String = (0..1005)
        .map(|index| format!("t\tcwd\told/{index}\tunknown script\n"))
        .collect();
    write(&fixture.home, ".canif/.misses.log", &prefill);
    let output = fixture.canif(&["run", "nothing"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!stderr(&output).contains("miss log"), "{}", stderr(&output));
    let lines = miss_lines(&fixture);
    assert_eq!(lines.len(), 1000);
    assert_eq!(lines[0], "t\tcwd\told/6\tunknown script");
    assert_eq!(lines[998], "t\tcwd\told/1004\tunknown script");
    assert_eq!(
        miss_fields(&fixture, &lines[999]),
        ["nothing", "unknown script"]
    );
}

#[test]
fn misses_groups_by_name() {
    let fixture = listed_fixture();
    let output = fixture.canif(&["misses"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "no misses recorded\n");

    for args in [
        &["run", "cargo/tets", "core"][..],
        &["run", "cargo/test", "nope"],
        &["run", "cargo/tets"],
        &["run", "cargo/test", "core", "--count", "9"],
        &["run", "cargo/test", "nope"],
        &["run", "cargo/tets"],
    ] {
        assert_eq!(fixture.canif(args).status.code(), Some(2));
    }
    let output = fixture.canif(&["misses"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "3  cargo/test
     2  arg `crate`: `nope` is not one of core|cli
     1  arg `count`: 9 is out of range, must be in 1..=5
3  cargo/tets
     3  unknown script
"
    );
}

#[test]
fn unwritable_miss_log_only_warns() {
    let fixture = listed_fixture();
    // A directory where the miss log should be makes every write fail.
    fs::create_dir_all(misses_path(&fixture)).unwrap();
    let output = fixture.canif(&["run", "echo/all", "nope", "a.rs"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let err = stderr(&output).replace("\r\n", "\n");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 3, "{err}");
    assert!(
        lines[0].starts_with("canif: cannot write miss log "),
        "{err}"
    );
    assert_eq!(
        lines[1],
        "canif: arg `mode`: `nope` is not one of fast|slow"
    );

    let output = fixture.canif(&["run", "greet"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn run_conflicted_name_fails() {
    let fixture = Fixture::new();
    fixture.write(".canif/build.sh", GREET);
    fixture.write(".canif/build.ps1", GREET);
    let output = fixture.canif(&["run", "build"]);
    assert_eq!(output.status.code(), Some(2));
    let text = stderr(&output);
    assert!(
        text.starts_with("canif: script `build` is defined more than once"),
        "{text}"
    );
}
