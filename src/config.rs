use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const FILE_NAME: &str = ".config.yml";
pub const DEFAULT_SCRIPT_EXTENSIONS: [&str; 3] = ["sh", "ps1", "py"];

/// Config of one `.canif` directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirConfig {
    pub walk: Walk,
    /// Lowercase extension (without dot) to program.
    pub interpreters: BTreeMap<String, String>,
}

/// Whether discovery keeps walking up after this directory (`root: true` stops it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Walk {
    #[default]
    Continue,
    Stop,
}

/// Config merged across all discovered directories, nearer overriding farther.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Lowercase extension (without dot) to program.
    pub interpreters: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{}: cannot read config: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: invalid config: {source}", path.display())]
    Yaml {
        path: PathBuf,
        source: Box<serde_saphyr::Error>,
    },
    #[error("{}: interpreter extension `{extension}` must be non-empty and without `.`", path.display())]
    InvalidExtension { path: PathBuf, extension: String },
    #[error("{}: interpreter extension `{extension}` is given more than once", path.display())]
    DuplicateExtension { path: PathBuf, extension: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    root: bool,
    #[serde(default)]
    interpreters: BTreeMap<String, String>,
}

/// Loads `<canif_dir>/.config.yml`; a missing or empty file is an empty config.
pub fn load(canif_dir: &Path) -> Result<DirConfig, ConfigError> {
    let path = canif_dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(DirConfig::default()),
        Err(source) => return Err(ConfigError::Read { path, source }),
    };
    if text.trim().is_empty() {
        return Ok(DirConfig::default());
    }
    let raw: RawConfig = match serde_saphyr::from_str(&text) {
        Ok(raw) => raw,
        Err(source) => {
            let source = Box::new(source);
            return Err(ConfigError::Yaml { path, source });
        }
    };
    let mut interpreters = BTreeMap::new();
    for (extension, program) in raw.interpreters {
        if extension.is_empty() || extension.contains('.') {
            return Err(ConfigError::InvalidExtension { path, extension });
        }
        if interpreters
            .insert(
                extension.to_ascii_lowercase(),
                resolve_program(canif_dir, program),
            )
            .is_some()
        {
            return Err(ConfigError::DuplicateExtension { path, extension });
        }
    }
    Ok(DirConfig {
        walk: if raw.root { Walk::Stop } else { Walk::Continue },
        interpreters,
    })
}

/// A program containing a path separator is a path, relative to the `.canif` dir that configured it.
/// Bare names are left for `PATH` lookup.
fn resolve_program(canif_dir: &Path, program: String) -> String {
    let path = Path::new(&program);
    let is_path = program.contains(['/', '\\']);
    if !is_path || path.is_absolute() || path.has_root() {
        return program;
    }
    canif_dir.join(path).to_string_lossy().into_owned()
}

impl Config {
    /// Merges per-directory configs given nearest first.
    pub fn merge<'config>(nearest_first: impl IntoIterator<Item = &'config DirConfig>) -> Self {
        let mut interpreters = BTreeMap::new();
        for dir_config in nearest_first {
            for (extension, program) in &dir_config.interpreters {
                interpreters
                    .entry(extension.clone())
                    .or_insert_with(|| program.clone());
            }
        }
        Self { interpreters }
    }

    pub fn is_script_extension(&self, extension: &str) -> bool {
        DEFAULT_SCRIPT_EXTENSIONS
            .iter()
            .any(|default| default.eq_ignore_ascii_case(extension))
            || self
                .interpreters
                .contains_key(&extension.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_text(text: &str) -> (tempfile::TempDir, Result<DirConfig, ConfigError>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), text).unwrap();
        let result = load(dir.path());
        (dir, result)
    }

    fn interpreters(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(extension, program)| (extension.to_string(), program.to_string()))
            .collect()
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), DirConfig::default());
    }

    #[test]
    fn empty_file_is_default() {
        for text in ["", "\n", "  \n"] {
            let (_dir, result) = load_text(text);
            assert_eq!(result.unwrap(), DirConfig::default(), "{text:?}");
        }
    }

    #[test]
    fn full_config() {
        let (_dir, result) =
            load_text("root: true\ninterpreters:\n  SH: /git/bin/bash.exe\n  js: node\n");
        let config = result.unwrap();
        assert_eq!(config.walk, Walk::Stop);
        assert_eq!(
            config.interpreters,
            interpreters(&[("js", "node"), ("sh", "/git/bin/bash.exe")])
        );
    }

    #[test]
    fn relative_interpreter_paths_resolve_against_canif_dir() {
        let (dir, result) =
            load_text("interpreters:\n  sh: tools/bash\n  js: node\n  py: ./py/run\n");
        let config = result.unwrap();
        let resolved = |relative: &str| dir.path().join(relative).to_string_lossy().into_owned();
        assert_eq!(config.interpreters["sh"], resolved("tools/bash"));
        assert_eq!(config.interpreters["py"], resolved("./py/run"));
        assert_eq!(config.interpreters["js"], "node");
    }

    #[test]
    fn rooted_interpreter_paths_are_kept() {
        let (_dir, result) =
            load_text("interpreters:\n  sh: /usr/bin/bash\n  ps1: C:/Tools/pwsh.exe\n");
        let config = result.unwrap();
        assert_eq!(config.interpreters["sh"], "/usr/bin/bash");
        if cfg!(windows) {
            assert_eq!(config.interpreters["ps1"], "C:/Tools/pwsh.exe");
        }
    }

    #[test]
    fn root_false() {
        let (_dir, result) = load_text("root: false\n");
        assert_eq!(result.unwrap().walk, Walk::Continue);
    }

    #[test]
    fn unknown_key_names_file() {
        let (dir, result) = load_text("roots: true\n");
        let Err(error @ ConfigError::Yaml { .. }) = result else {
            panic!("expected YAML error, got {result:?}");
        };
        let path = dir.path().join(FILE_NAME);
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}"
        );
    }

    #[test]
    fn invalid_yaml() {
        for text in [
            "interpreters: [a\n",
            "root: maybe\n",
            "interpreters: [a, b]\n",
        ] {
            let (_dir, result) = load_text(text);
            assert!(
                matches!(result, Err(ConfigError::Yaml { .. })),
                "{text:?}: {result:?}"
            );
        }
    }

    #[test]
    fn invalid_extension() {
        for text in [
            "interpreters:\n  .sh: bash\n",
            "interpreters:\n  '': bash\n",
        ] {
            let (_dir, result) = load_text(text);
            assert!(
                matches!(result, Err(ConfigError::InvalidExtension { .. })),
                "{text:?}: {result:?}"
            );
        }
    }

    #[test]
    fn duplicate_extension_across_case() {
        let (_dir, result) = load_text("interpreters:\n  sh: a\n  SH: b\n");
        assert!(matches!(
            result,
            Err(ConfigError::DuplicateExtension { .. })
        ));
    }

    #[test]
    fn merge_nearer_wins_per_key() {
        let near = DirConfig {
            walk: Walk::Continue,
            interpreters: interpreters(&[("sh", "near-bash")]),
        };
        let far = DirConfig {
            walk: Walk::Stop,
            interpreters: interpreters(&[("sh", "far-bash"), ("py", "python3")]),
        };
        let merged = Config::merge([&near, &far]);
        assert_eq!(
            merged.interpreters,
            interpreters(&[("py", "python3"), ("sh", "near-bash")])
        );
    }

    #[test]
    fn script_extensions() {
        let config = Config {
            interpreters: interpreters(&[("js", "node")]),
        };
        for extension in ["sh", "SH", "ps1", "Py", "js", "JS"] {
            assert!(config.is_script_extension(extension), "{extension}");
        }
        for extension in ["txt", "", "md"] {
            assert!(!config.is_script_extension(extension), "{extension}");
        }
        assert!(!Config::default().is_script_extension("js"));
    }
}
