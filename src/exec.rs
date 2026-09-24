use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
#[cfg(any(windows, test))]
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::binding::Bindings;
use crate::config;
use crate::discovery::{Origin, Script};
use crate::output::{self, Trim};

const ENV_PREFIX: &str = "CANIF_";
const POWERSHELL_FLAGS: [&str; 3] = ["-NoProfile", "-NonInteractive", "-File"];

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[cfg(windows)]
    #[error(
        "Git Bash not found for .sh scripts, set `interpreters: {{ sh: <path to bash.exe> }}` in .canif/.config.yml"
    )]
    GitBashNotFound,
    #[error(
        "no interpreter for .{extension} scripts, set `interpreters: {{ {extension}: <program> }}` in .canif/.config.yml"
    )]
    NoInterpreter { extension: String },
    #[error(
        "interpreter `{program}` for .{extension} scripts not found, install it or set `interpreters: {{ {extension}: <program> }}` in .canif/.config.yml"
    )]
    InterpreterNotFound { program: String, extension: String },
    #[error("cannot start `{program}`: {source}")]
    Spawn { program: String, source: io::Error },
    #[error("cannot wait for the script: {source}")]
    Wait { source: io::Error },
}

/// Runs the script in the inherited cwd with its output relayed and logged, returning the exit code to propagate.
pub fn run(
    script: &Script,
    origin: &Origin,
    config: &config::Config,
    bindings: &Bindings,
    trim: Trim,
    logs_dir: Option<&Path>,
) -> Result<i32, ExecError> {
    let program = interpreter(&script.extension, config)?;
    let mut command = Command::new(&program);
    if script.extension == "ps1" {
        command.args(POWERSHELL_FLAGS);
    }
    command
        .arg(portable_path(&script.path))
        .args(&bindings.argv);
    for (name, _) in std::env::vars_os() {
        if is_canif_var(&name) {
            command.env_remove(name);
        }
    }
    command
        .env("CANIF_DIR", portable_path(&origin.canif_dir))
        .env("CANIF_PROJECT", portable_path(&origin.project_dir))
        .envs(bindings.env.iter().map(|(name, value)| (name, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().map_err(|source| {
        let program = program.to_string_lossy().into_owned();
        if source.kind() == io::ErrorKind::NotFound {
            ExecError::InterpreterNotFound {
                program,
                extension: script.extension.clone(),
            }
        } else {
            ExecError::Spawn { program, source }
        }
    })?;
    let status = output::relay(child, trim, logs_dir, &script.name)
        .map_err(|source| ExecError::Wait { source })?;
    Ok(status.code().unwrap_or(1))
}

fn interpreter(extension: &str, config: &config::Config) -> Result<OsString, ExecError> {
    if let Some(program) = config.interpreters.get(extension) {
        return Ok(program.into());
    }
    match extension {
        "sh" => default_bash(),
        "ps1" => Ok("pwsh".into()),
        "py" if cfg!(windows) => Ok("py".into()),
        "py" => Ok("python3".into()),
        _ => Err(ExecError::NoInterpreter {
            extension: extension.to_owned(),
        }),
    }
}

#[cfg(windows)]
fn default_bash() -> Result<OsString, ExecError> {
    find_git_bash()
        .map(PathBuf::into_os_string)
        .ok_or(ExecError::GitBashNotFound)
}

#[cfg(not(windows))]
fn default_bash() -> Result<OsString, ExecError> {
    Ok("bash".into())
}

/// Never a bare `bash`: on Windows it can resolve to the WSL launcher in System32.
#[cfg(windows)]
fn find_git_bash() -> Option<PathBuf> {
    let from_git = Command::new("git")
        .arg("--exec-path")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|exec_path| git_bash_from_exec_path(&exec_path));
    let program_files = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(|dir| PathBuf::from(dir).join(r"Programs\Git\bin\bash.exe"));
    from_git
        .into_iter()
        .chain([program_files])
        .chain(local_app_data)
        .find(|candidate| candidate.is_file())
}

/// Maps `git --exec-path` output (`<root>/mingw64/libexec/git-core`) to `<root>/bin/bash.exe`.
#[cfg(any(windows, test))]
pub fn git_bash_from_exec_path(exec_path: &str) -> Option<PathBuf> {
    let libexec = Path::new(exec_path.trim()).ancestors().find(|dir| {
        dir.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("libexec"))
    })?;
    let root = libexec
        .parent()?
        .parent()
        .filter(|root| !root.as_os_str().is_empty())?;
    Some(root.join("bin").join("bash.exe"))
}

fn is_canif_var(name: &OsStr) -> bool {
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(ENV_PREFIX)
}

/// Forward slashes on Windows, so scripts can append `/_lib/x.sh` to `CANIF_DIR` safely.
pub fn portable_path(path: &Path) -> OsString {
    if cfg!(windows) {
        path.to_string_lossy().replace('\\', "/").into()
    } else {
        path.as_os_str().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_bash(root: &str) -> Option<PathBuf> {
        Some(PathBuf::from(root).join("bin").join("bash.exe"))
    }

    #[test]
    fn git_bash_from_standard_exec_path() {
        assert_eq!(
            git_bash_from_exec_path("C:/Program Files/Git/mingw64/libexec/git-core\n"),
            expected_bash("C:/Program Files/Git")
        );
        assert_eq!(
            git_bash_from_exec_path("D:/tools/git/mingw32/libexec/git-core\r\n"),
            expected_bash("D:/tools/git")
        );
    }

    #[test]
    fn git_bash_from_unexpected_exec_path() {
        assert_eq!(git_bash_from_exec_path(""), None);
        assert_eq!(git_bash_from_exec_path("/libexec/git-core"), None);
        assert_eq!(git_bash_from_exec_path("mingw64/libexec/git-core"), None);
        assert_eq!(git_bash_from_exec_path("C:/Git/cmd"), None);
    }

    #[test]
    fn canif_vars_match_case_insensitively() {
        assert!(is_canif_var(OsStr::new("CANIF_FILTER")));
        assert!(is_canif_var(OsStr::new("canif_filter")));
        assert!(!is_canif_var(OsStr::new("CANIFX")));
        assert!(!is_canif_var(OsStr::new("PATH")));
    }

    #[test]
    fn config_override_wins() {
        let mut config = config::Config::default();
        config
            .interpreters
            .insert("sh".to_owned(), "my-bash".to_owned());
        config
            .interpreters
            .insert("js".to_owned(), "node".to_owned());
        assert_eq!(interpreter("sh", &config).unwrap(), "my-bash");
        assert_eq!(interpreter("js", &config).unwrap(), "node");
        assert_eq!(interpreter("ps1", &config).unwrap(), "pwsh");
        assert!(matches!(
            interpreter("rb", &config),
            Err(ExecError::NoInterpreter { .. })
        ));
    }
}
