use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::{self, ConfigError, Walk};

pub const DIR_NAME: &str = ".canif";

#[derive(Debug)]
pub struct Discovery {
    /// Contributing directories, nearest first.
    pub origins: Vec<Origin>,
    pub config: config::Config,
    /// Winning scripts, sorted by name.
    pub scripts: Vec<Script>,
    pub unknown: Vec<UnknownFile>,
    pub conflicts: Vec<Conflict>,
    /// Scripts hidden by a same-named entry in a nearer dir, sorted by name then origin.
    pub shadowed: Vec<Shadowed>,
    /// Unreadable entries and invalid configs; discovery carries on past them.
    pub problems: Vec<DiscoveryError>,
}

impl Discovery {
    /// Fails with the first problem, for commands that must not act on a partial view.
    pub fn require_clean(mut self) -> Result<Self, DiscoveryError> {
        if self.problems.is_empty() {
            Ok(self)
        } else {
            Err(self.problems.remove(0))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub canif_dir: PathBuf,
    pub project_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub name: String,
    pub path: PathBuf,
    /// Lowercase, without dot.
    pub extension: String,
    /// Index into [`Discovery::origins`].
    pub origin: usize,
}

/// A file with no script extension, or without extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFile {
    pub path: PathBuf,
    pub origin: usize,
}

/// Several files with the same script name inside one `.canif` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub name: String,
    pub paths: Vec<PathBuf>,
    pub origin: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed {
    pub script: Script,
    /// Index into [`Discovery::origins`] of the dir claiming the name, by a script or a conflict.
    pub by: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Config(#[from] ConfigError),
}

#[derive(Default)]
struct Listing {
    candidates: BTreeMap<String, Vec<(PathBuf, String)>>,
    unknown: Vec<PathBuf>,
    problems: Vec<DiscoveryError>,
}

pub fn discover(cwd: &Path, home: Option<&Path>) -> Discovery {
    discover_in(cwd.ancestors(), home)
}

fn discover_in<'path>(
    walk: impl IntoIterator<Item = &'path Path>,
    home: Option<&Path>,
) -> Discovery {
    let mut problems = Vec::new();
    let mut load_config = |canif_dir: &Path| {
        config::load(canif_dir).unwrap_or_else(|error| {
            problems.push(DiscoveryError::Config(error));
            config::DirConfig::default()
        })
    };
    let mut origins = Vec::new();
    let mut dir_configs = Vec::new();
    for project_dir in walk {
        let canif_dir = project_dir.join(DIR_NAME);
        if !canif_dir.is_dir() {
            continue;
        }
        let dir_config = load_config(&canif_dir);
        let walk = dir_config.walk;
        dir_configs.push(dir_config);
        origins.push(Origin {
            canif_dir,
            project_dir: project_dir.to_path_buf(),
        });
        if walk == Walk::Stop {
            break;
        }
    }
    if let Some(home) = home {
        let canif_dir = home.join(DIR_NAME);
        if canif_dir.is_dir() && !is_listed(&origins, &canif_dir) {
            dir_configs.push(load_config(&canif_dir));
            origins.push(Origin {
                canif_dir,
                project_dir: home.to_path_buf(),
            });
        }
    }
    let config = config::Config::merge(&dir_configs);

    let mut claimed = BTreeMap::new();
    let mut scripts = Vec::new();
    let mut unknown = Vec::new();
    let mut conflicts = Vec::new();
    let mut shadowed = Vec::new();
    for (origin, Origin { canif_dir, .. }) in origins.iter().enumerate() {
        let mut listing = Listing::default();
        list_dir(canif_dir, "", &config, &mut listing);
        problems.append(&mut listing.problems);
        unknown.extend(
            listing
                .unknown
                .into_iter()
                .map(|path| UnknownFile { path, origin }),
        );
        for (name, mut found) in listing.candidates {
            let claimant = *claimed.entry(name.clone()).or_insert(origin);
            if found.len() > 1 {
                let paths = found.into_iter().map(|(path, _)| path).collect();
                conflicts.push(Conflict {
                    name,
                    paths,
                    origin,
                });
            } else if let Some((path, extension)) = found.pop() {
                let script = Script {
                    name,
                    path,
                    extension,
                    origin,
                };
                if claimant == origin {
                    scripts.push(script);
                } else {
                    shadowed.push(Shadowed {
                        script,
                        by: claimant,
                    });
                }
            }
        }
    }
    scripts.sort_by(|left, right| left.name.cmp(&right.name));
    // Stable: equal names keep origin order.
    shadowed.sort_by(|left, right| left.script.name.cmp(&right.script.name));

    Discovery {
        origins,
        config,
        scripts,
        unknown,
        conflicts,
        shadowed,
        problems,
    }
}

fn is_listed(origins: &[Origin], canif_dir: &Path) -> bool {
    let target = canonicalize(canif_dir);
    origins
        .iter()
        .any(|origin| canonicalize(&origin.canif_dir) == target)
}

/// Falls back to the path as given, so an uncanonicalizable dir compares by its spelling.
fn canonicalize(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> DiscoveryError {
    |source| DiscoveryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn list_dir(dir: &Path, name_prefix: &str, config: &config::Config, listing: &mut Listing) {
    let entries = fs::read_dir(dir).and_then(|entries| {
        entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()
    });
    let mut entries = match entries {
        Ok(entries) => entries,
        Err(error) => {
            listing.problems.push(io_error(dir)(error));
            return;
        }
    };
    entries.sort();
    for path in entries {
        let Some(file_name) = path.file_name() else {
            continue;
        };
        if file_name.to_string_lossy().starts_with(['.', '_']) {
            continue;
        }
        let Some(file_name) = file_name.to_str() else {
            listing.unknown.push(path);
            continue;
        };
        let link_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                listing.problems.push(io_error(&path)(error));
                continue;
            }
        };
        let is_link = link_metadata.file_type().is_symlink();
        // `fs::metadata` follows the link: a symlinked dir is skipped, a symlinked file is a script.
        if is_link && fs::metadata(&path).is_ok_and(|target| target.is_dir()) {
            continue;
        }
        if link_metadata.is_dir() {
            let prefix = format!("{name_prefix}{file_name}/");
            list_dir(&path, &prefix, config, listing);
            continue;
        }
        let file_name = Path::new(file_name);
        let stem = file_name.file_stem().and_then(|stem| stem.to_str());
        let extension = file_name
            .extension()
            .and_then(|extension| extension.to_str());
        match (stem, extension) {
            (Some(stem), Some(extension)) if config.is_script_extension(extension) => {
                let name = format!("{name_prefix}{stem}");
                let extension = extension.to_ascii_lowercase();
                listing
                    .candidates
                    .entry(name)
                    .or_default()
                    .push((path, extension));
            }
            _ => listing.unknown.push(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Walks only inside the system temp dir, so a real `.canif` above it (e.g. `~/.canif`) can't leak in.
    fn discover(cwd: &Path, home: Option<&Path>) -> Result<Discovery, DiscoveryError> {
        let boundary = std::env::temp_dir();
        let walk = cwd
            .ancestors()
            .take_while(|dir| dir.starts_with(&boundary) && *dir != boundary);
        discover_in(walk, home).require_clean()
    }

    #[test]
    fn public_discover_walks_every_ancestor() {
        let tmp = fixture();
        touch(tmp.path(), ".canif/ok.sh");
        let cwd = mkdir(tmp.path(), "a/b");
        let discovery = super::discover(&cwd, None);
        assert_eq!(discovery.origins[0].project_dir, tmp.path());
    }

    fn write(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn touch(root: &Path, relative: &str) {
        write(root, relative, "");
    }

    fn mkdir(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn names(discovery: &Discovery) -> Vec<&str> {
        discovery
            .scripts
            .iter()
            .map(|script| script.name.as_str())
            .collect()
    }

    fn script<'discovery>(discovery: &'discovery Discovery, name: &str) -> &'discovery Script {
        discovery
            .scripts
            .iter()
            .find(|script| script.name == name)
            .unwrap_or_else(|| panic!("no script {name}"))
    }

    fn project_dirs(discovery: &Discovery) -> Vec<&Path> {
        discovery
            .origins
            .iter()
            .map(|origin| origin.project_dir.as_path())
            .collect()
    }

    fn unknown_paths(discovery: &Discovery) -> Vec<&Path> {
        discovery
            .unknown
            .iter()
            .map(|file| file.path.as_path())
            .collect()
    }

    #[test]
    fn single_dir() {
        let tmp = fixture();
        let project = tmp.path().join("p");
        touch(&project, ".canif/build.sh");
        touch(&project, ".canif/deploy.ps1");
        let cwd = mkdir(&project, "src/deep");
        let discovery = discover(&cwd, None).unwrap();
        assert_eq!(
            discovery.origins,
            [Origin {
                canif_dir: project.join(DIR_NAME),
                project_dir: project.clone(),
            }]
        );
        assert_eq!(names(&discovery), ["build", "deploy"]);
        let deploy = script(&discovery, "deploy");
        assert_eq!(deploy.path, project.join(".canif/deploy.ps1"));
        assert_eq!(deploy.extension, "ps1");
        assert_eq!(deploy.origin, 0);
        assert!(discovery.unknown.is_empty());
        assert!(discovery.conflicts.is_empty());
    }

    #[test]
    fn nearest_dir_wins() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        touch(&outer, ".canif/build.sh");
        touch(&outer, ".canif/lint.sh");
        touch(&inner, ".canif/build.py");
        let discovery = discover(&inner, None).unwrap();
        assert_eq!(project_dirs(&discovery), [inner.as_path(), outer.as_path()]);
        assert_eq!(names(&discovery), ["build", "lint"]);
        let build = script(&discovery, "build");
        assert_eq!(build.path, inner.join(".canif/build.py"));
        assert_eq!(build.origin, 0);
        assert_eq!(script(&discovery, "lint").origin, 1);
        assert_eq!(
            discovery.shadowed,
            [Shadowed {
                script: Script {
                    name: "build".to_owned(),
                    path: outer.join(".canif").join("build.sh"),
                    extension: "sh".to_owned(),
                    origin: 1,
                },
                by: 0,
            }]
        );
    }

    #[test]
    fn root_stops_walk_but_keeps_home() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        let home = tmp.path().join("home");
        touch(&outer, ".canif/outer.sh");
        write(&inner, ".canif/.config.yml", "root: true\n");
        touch(&inner, ".canif/inner.sh");
        touch(&home, ".canif/home.sh");
        let discovery = discover(&inner, Some(&home)).unwrap();
        assert_eq!(project_dirs(&discovery), [inner.as_path(), home.as_path()]);
        assert_eq!(names(&discovery), ["home", "inner"]);
    }

    #[test]
    fn home_counted_once_when_walk_reaches_it() {
        let tmp = fixture();
        let home = tmp.path().join("home");
        let project = home.join("project");
        touch(&home, ".canif/home.sh");
        touch(&project, ".canif/project.sh");
        let aliased_home = project.join("..");
        for home_arg in [home.as_path(), aliased_home.as_path()] {
            let discovery = discover(&project, Some(home_arg)).unwrap();
            assert_eq!(
                project_dirs(&discovery),
                [project.as_path(), home.as_path()]
            );
            assert_eq!(names(&discovery), ["home", "project"]);
        }
    }

    #[test]
    fn skips_dot_and_underscore_entries() {
        let tmp = fixture();
        write(tmp.path(), ".canif/.config.yml", "root: false\n");
        touch(tmp.path(), ".canif/.hidden.sh");
        touch(tmp.path(), ".canif/_private.sh");
        touch(tmp.path(), ".canif/_lib/common.sh");
        touch(tmp.path(), ".canif/.git/hook.sh");
        touch(tmp.path(), ".canif/tools/_helper.sh");
        touch(tmp.path(), ".canif/ok.sh");
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["ok"]);
        assert!(discovery.unknown.is_empty());
    }

    #[test]
    fn unknown_and_extensionless_files() {
        let tmp = fixture();
        let canif = tmp.path().join(DIR_NAME);
        touch(tmp.path(), ".canif/Makefile");
        touch(tmp.path(), ".canif/notes/readme.md");
        touch(tmp.path(), ".canif/run.sh");
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["run"]);
        assert_eq!(
            unknown_paths(&discovery),
            [
                canif.join("Makefile"),
                canif.join("notes").join("readme.md")
            ]
        );
        assert!(discovery.unknown.iter().all(|file| file.origin == 0));
    }

    #[test]
    fn config_adds_script_extension_for_all_dirs() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        write(&outer, ".canif/.config.yml", "interpreters:\n  JS: node\n");
        touch(&inner, ".canif/tool.js");
        touch(&inner, ".canif/other.rb");
        let discovery = discover(&inner, None).unwrap();
        assert_eq!(names(&discovery), ["tool"]);
        assert_eq!(script(&discovery, "tool").extension, "js");
        assert_eq!(
            unknown_paths(&discovery),
            [inner.join(".canif").join("other.rb")]
        );
    }

    #[test]
    fn interpreters_merge_nearer_wins() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        write(
            &outer,
            ".canif/.config.yml",
            "interpreters:\n  sh: far-bash\n  py: python3\n",
        );
        write(
            &inner,
            ".canif/.config.yml",
            "interpreters:\n  sh: near-bash\n",
        );
        let discovery = discover(&inner, None).unwrap();
        let interpreters: Vec<(&str, &str)> = discovery
            .config
            .interpreters
            .iter()
            .map(|(extension, program)| (extension.as_str(), program.as_str()))
            .collect();
        assert_eq!(interpreters, [("py", "python3"), ("sh", "near-bash")]);
    }

    #[test]
    fn extension_is_case_insensitive() {
        let tmp = fixture();
        touch(tmp.path(), ".canif/X.SH");
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["X"]);
        assert_eq!(script(&discovery, "X").extension, "sh");
    }

    #[test]
    fn nested_names_use_slashes() {
        let tmp = fixture();
        touch(tmp.path(), ".canif/cargo/test.sh");
        touch(tmp.path(), ".canif/a/b/c.py");
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["a/b/c", "cargo/test"]);
    }

    #[test]
    fn same_name_in_one_dir_is_a_conflict() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        touch(&outer, ".canif/build.sh");
        touch(&inner, ".canif/build.sh");
        touch(&inner, ".canif/build.ps1");
        touch(&inner, ".canif/other.sh");
        let discovery = discover(&inner, None).unwrap();
        assert_eq!(names(&discovery), ["other"]);
        assert_eq!(
            discovery.conflicts,
            [Conflict {
                name: "build".to_owned(),
                paths: vec![
                    inner.join(".canif").join("build.ps1"),
                    inner.join(".canif").join("build.sh"),
                ],
                origin: 0,
            }]
        );
        let hidden: Vec<(&Path, usize)> = discovery
            .shadowed
            .iter()
            .map(|entry| (entry.script.path.as_path(), entry.by))
            .collect();
        assert_eq!(
            hidden,
            [(outer.join(".canif").join("build.sh").as_path(), 0)]
        );
    }

    #[test]
    fn invalid_config_is_an_error() {
        let tmp = fixture();
        write(tmp.path(), ".canif/.config.yml", "bogus: 1\n");
        let result = discover(tmp.path(), None);
        assert!(
            matches!(
                result,
                Err(DiscoveryError::Config(ConfigError::Yaml { .. }))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn invalid_home_config_is_an_error() {
        let tmp = fixture();
        let home = tmp.path().join("home");
        write(&home, ".canif/.config.yml", "root: [\n");
        let cwd = mkdir(tmp.path(), "work");
        let result = discover(&cwd, Some(&home));
        assert!(
            matches!(result, Err(DiscoveryError::Config(_))),
            "{result:?}"
        );
    }

    #[test]
    fn problems_are_collected_without_stopping() {
        let tmp = fixture();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        let home = tmp.path().join("home");
        write(&inner, ".canif/.config.yml", "bogus: 1\n");
        touch(&inner, ".canif/near.sh");
        write(&home, ".canif/.config.yml", "root: [\n");
        touch(&outer, ".canif/far.sh");
        let boundary = std::env::temp_dir();
        let walk = inner
            .ancestors()
            .take_while(|dir| dir.starts_with(&boundary) && *dir != boundary);
        let discovery = discover_in(walk, Some(&home));
        assert_eq!(names(&discovery), ["far", "near"]);
        assert_eq!(discovery.problems.len(), 2, "{:?}", discovery.problems);
        assert!(
            discovery
                .problems
                .iter()
                .all(|problem| matches!(problem, DiscoveryError::Config(_)))
        );
        assert!(discovery.require_clean().is_err());
    }

    /// A link back to `.canif` would recurse forever if followed.
    #[test]
    fn symlinked_dirs_are_not_followed() {
        let tmp = fixture();
        touch(tmp.path(), ".canif/sub/ok.sh");
        let canif = tmp.path().join(DIR_NAME);
        let link = canif.join("sub").join("loop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&canif, &link).unwrap();
        // Junctions need no privilege, unlike Windows symlinks, and std reports both as symlinks.
        #[cfg(windows)]
        assert!(
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&canif)
                .output()
                .unwrap()
                .status
                .success()
        );
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["sub/ok"]);
        assert!(discovery.unknown.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_are_scripts() {
        let tmp = fixture();
        touch(tmp.path(), ".canif/sub/ok.sh");
        let canif = tmp.path().join(DIR_NAME);
        std::os::unix::fs::symlink(canif.join("sub/ok.sh"), canif.join("alias.sh")).unwrap();
        let discovery = discover(tmp.path(), None).unwrap();
        assert_eq!(names(&discovery), ["alias", "sub/ok"]);
    }

    #[test]
    fn nothing_found_is_empty() {
        let tmp = fixture();
        let cwd = mkdir(tmp.path(), "a/b");
        let home = mkdir(tmp.path(), "home");
        let discovery = discover(&cwd, Some(&home)).unwrap();
        assert!(discovery.origins.is_empty());
        assert!(discovery.scripts.is_empty());
        assert!(discovery.unknown.is_empty());
        assert!(discovery.conflicts.is_empty());
        assert!(discovery.config.interpreters.is_empty());
    }

    #[test]
    fn canif_file_is_not_a_dir() {
        let tmp = fixture();
        touch(tmp.path(), ".canif");
        let discovery = discover(tmp.path(), None).unwrap();
        assert!(discovery.origins.is_empty());
    }
}
