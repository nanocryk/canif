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
        let home = root.path().join("home");
        let project = root.path().join("project");
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
        "usage: canif run echo/all <mode:fast|slow> <files>... [--filter <filter>] [--release]
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
usage: canif run echo/all <mode:fast|slow> <files>... [--filter <filter>] [--release]
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
