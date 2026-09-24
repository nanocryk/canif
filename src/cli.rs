use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::binding::{self, BindError};
use crate::discovery::{self, Discovery, DiscoveryError, Script};
use crate::exec::{self, ExecError};
use crate::header::{self, ArgKind, ArgSpec, Arity, Confirm, Header, HeaderError, Presence};
use crate::misses;
use crate::output::{LOGS_DIR, Trim};
use crate::suggest;
use crate::usage;

/// Run reviewed scripts from `.canif/` directories with validated args.
#[derive(Debug, Parser)]
#[command(name = "canif", version)]
pub struct Cli {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List scripts; with terms, only those whose name or description contains any term.
    List { terms: Vec<String> },
    /// Show a script's usage, description, source path and arg rules.
    Show { name: String },
    /// Validate args against the script header, then run it.
    // Without this, a `--help` right after the name is taken by clap instead of the script.
    #[command(disable_help_flag = true)]
    Run(Invocation),
    /// Same as `run`, for scripts whose header sets `confirm: true`.
    #[command(disable_help_flag = true)]
    Confirm(Invocation),
    /// Print the miss log (unknown names, validation failures) grouped by script name.
    Misses,
    /// Validate every script header and report conflicts.
    Check,
    /// Print the reference for writing scripts: header format, arg kinds, env, config.
    Guide,
}

#[derive(Debug, Args)]
pub struct Invocation {
    /// Print only the first <n> lines of output (overrides the header)
    #[arg(long, value_name = "n")]
    head: Option<NonZeroUsize>,
    /// Print only the last <n> lines of output (overrides the header)
    #[arg(long, value_name = "n")]
    tail: Option<NonZeroUsize>,
    /// Script name, then its args.
    // One positional, so that options after the name (even `--head`) go to the script.
    #[arg(
        required = true,
        value_name = "NAME [ARGS]",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    command: Vec<String>,
}

impl Invocation {
    fn name(&self) -> &str {
        &self.command[0]
    }

    fn args(&self) -> &[String] {
        &self.command[1..]
    }
}

pub const GUIDE: &str = include_str!("guide.md");

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("cannot read the current directory: {0}")]
    CurrentDir(io::Error),
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error("{}", unknown_script_text(name, suggestion.as_deref()))]
    UnknownScript {
        name: String,
        suggestion: Option<String>,
    },
    #[error(
        "script `{name}` is defined more than once: {}, run `canif check`",
        joined_paths(paths)
    )]
    Conflict { name: String, paths: Vec<PathBuf> },
    #[error("{}: cannot read script: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Header {
        path: PathBuf,
        source: Box<HeaderError>,
    },
    #[error("{}", wrong_subcommand_text(name, *confirm))]
    WrongSubcommand { name: String, confirm: Confirm },
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("cannot read miss log {}: {source}", path.display())]
    MissLog { path: PathBuf, source: io::Error },
}

/// Runs the parsed command and returns the process exit code.
pub fn execute(cli: Cli) -> Result<i32, CliError> {
    match cli.action {
        Action::List { terms } => {
            print!("{}", list(&discover()?, &terms));
            Ok(0)
        }
        Action::Show { name } => {
            let discovery = discover()?;
            let script = find(&discovery, &name)?;
            print!("{}", show(script, &load_header(script)?));
            Ok(0)
        }
        Action::Run(invocation) => run(&invocation, Confirm::NotRequired),
        Action::Confirm(invocation) => run(&invocation, Confirm::Required),
        Action::Misses => {
            print!("{}", misses_summary()?);
            Ok(0)
        }
        Action::Check => {
            let report = check(&discover_all()?);
            print!("{}", report.text);
            Ok(match report.outcome {
                Outcome::Clean => 0,
                Outcome::Problems => 1,
            })
        }
        Action::Guide => {
            print!("{GUIDE}");
            Ok(0)
        }
    }
}

/// Shared by `run` and `confirm`: `invoked` is the confirm level the subcommand serves.
fn run(invocation: &Invocation, invoked: Confirm) -> Result<i32, CliError> {
    let home = std::env::home_dir();
    let result = run_script(invocation, invoked, home.as_deref());
    let miss_reason = match &result {
        Err(CliError::UnknownScript { .. }) => Some("unknown script".to_owned()),
        Err(CliError::Bind(error)) => Some(error.reason()),
        _ => None,
    };
    if let Some(reason) = miss_reason {
        misses::record(
            home.as_deref(),
            invocation.name(),
            invocation.args(),
            reason,
        );
    }
    result
}

fn run_script(
    invocation: &Invocation,
    invoked: Confirm,
    home: Option<&Path>,
) -> Result<i32, CliError> {
    let discovery = discover()?;
    let script = find(&discovery, invocation.name())?;
    let header = load_header(script)?;
    if header.confirm != invoked {
        return Err(CliError::WrongSubcommand {
            name: script.name.clone(),
            confirm: header.confirm,
        });
    }
    let bindings = binding::bind(&script.name, &header, invocation.args())?;
    let origin = &discovery.origins[script.origin];
    let trim = header.output.overridden_by(Trim {
        head: invocation.head,
        tail: invocation.tail,
    });
    let logs_dir = home.map(|home| home.join(discovery::DIR_NAME).join(LOGS_DIR));
    Ok(exec::run(
        script,
        origin,
        &discovery.config,
        &bindings,
        trim,
        logs_dir.as_deref(),
    )?)
}

fn misses_summary() -> Result<String, CliError> {
    let Some(home) = std::env::home_dir() else {
        return Ok(misses::summary(""));
    };
    let path = misses::log_path(&home);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(misses::summary(&String::from_utf8_lossy(&bytes))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(misses::summary("")),
        Err(source) => Err(CliError::MissLog { path, source }),
    }
}

/// Discovery that fails on the first unreadable entry or invalid config.
fn discover() -> Result<Discovery, CliError> {
    Ok(discover_all()?.require_clean()?)
}

/// Discovery that keeps its problems, for `check` to report them all.
fn discover_all() -> Result<Discovery, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let home = std::env::home_dir();
    Ok(discovery::discover(&cwd, home.as_deref()))
}

fn load_header(script: &Script) -> Result<Header, CliError> {
    let path = script.path.clone();
    let source = match std::fs::read_to_string(&script.path) {
        Ok(source) => source,
        Err(source) => return Err(CliError::Read { path, source }),
    };
    header::parse(&source).map_err(|source| CliError::Header {
        path,
        source: Box::new(source),
    })
}

fn find<'discovery>(
    discovery: &'discovery Discovery,
    name: &str,
) -> Result<&'discovery Script, CliError> {
    if let Some(script) = discovery.scripts.iter().find(|script| script.name == name) {
        return Ok(script);
    }
    if let Some(conflict) = discovery
        .conflicts
        .iter()
        .find(|conflict| conflict.name == name)
    {
        return Err(CliError::Conflict {
            name: name.to_owned(),
            paths: conflict.paths.clone(),
        });
    }
    let names: Vec<&str> = discovery
        .scripts
        .iter()
        .map(|script| script.name.as_str())
        .collect();
    Err(CliError::UnknownScript {
        name: name.to_owned(),
        suggestion: suggest::closest(name, &names).map(str::to_owned),
    })
}

fn unknown_script_text(name: &str, suggestion: Option<&str>) -> String {
    match suggestion {
        Some(suggestion) => format!("unknown script `{name}`, did you mean `{suggestion}`?"),
        None => format!("unknown script `{name}`, run `canif list` to see available scripts"),
    }
}

fn wrong_subcommand_text(name: &str, confirm: Confirm) -> String {
    let command = format!("canif {} {name}", confirm.subcommand());
    match confirm {
        Confirm::Required => format!("script `{name}` has `confirm: true`, run `{command}`"),
        Confirm::NotRequired => {
            format!("script `{name}` does not have `confirm: true`, run `{command}`")
        }
    }
}

fn joined_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn list(discovery: &Discovery, terms: &[String]) -> String {
    if discovery.scripts.is_empty() {
        return "no .canif scripts found\n".to_owned();
    }
    let terms: Vec<String> = terms.iter().map(|term| term.to_lowercase()).collect();
    let matches = |text: &str| {
        let text = text.to_lowercase();
        terms.iter().any(|term| text.contains(term.as_str()))
    };
    let mut output = String::new();
    for (origin_index, origin) in discovery.origins.iter().enumerate() {
        let mut lines = Vec::new();
        for script in discovery
            .scripts
            .iter()
            .filter(|script| script.origin == origin_index)
        {
            let header = load_header(script).ok();
            let description = header.as_ref().map(|header| header.description.as_str());
            if !terms.is_empty() && !matches(&script.name) && !description.is_some_and(matches) {
                continue;
            }
            lines.push(match &header {
                Some(header) => format!(
                    "  {}  # {}{}",
                    usage::line(&script.name, header),
                    match header.confirm {
                        Confirm::Required => "[confirm] ",
                        Confirm::NotRequired => "",
                    },
                    header.description
                ),
                None => format!("  {}  # invalid header, run canif check", script.name),
            });
        }
        if lines.is_empty() {
            continue;
        }
        output.push_str(&format!("{}:\n", origin.canif_dir.display()));
        for line in lines {
            output.push_str(&line);
            output.push('\n');
        }
    }
    if output.is_empty() {
        return format!("no script matches: {}\n", terms.join(" "));
    }
    output
}

fn show(script: &Script, header: &Header) -> String {
    let mut lines = vec![
        format!("usage: {}", usage::command(&script.name, header)),
        format!("description: {}", header.description),
        format!("source: {}", script.path.display()),
    ];
    if header.confirm == Confirm::Required {
        lines.push("confirm: required".to_owned());
    }
    let trim_parts: Vec<String> = [("head", header.output.head), ("tail", header.output.tail)]
        .into_iter()
        .filter_map(|(key, count)| count.map(|count| format!("{key} {count}")))
        .collect();
    if !trim_parts.is_empty() {
        lines.push(format!("output: {}", trim_parts.join(", ")));
    }
    for (name, spec) in &header.args {
        lines.push(format!("arg {name}: {}", arg_details(spec)));
    }
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn arg_details(spec: &ArgSpec) -> String {
    let mut parts = vec![kind_text(&spec.kind)];
    parts.push(
        match spec.presence {
            Presence::Required => "required",
            Presence::Optional => "optional",
        }
        .to_owned(),
    );
    if spec.arity == Arity::Many {
        parts.push("many".to_owned());
    }
    if let Some(default) = &spec.default {
        parts.push(format!("default: {default}"));
    }
    if let Some(help) = &spec.help {
        parts.push(format!("help: {help}"));
    }
    parts.join(", ")
}

fn kind_text(kind: &ArgKind) -> String {
    match kind {
        ArgKind::Choices(choices) => format!("choices {}", choices.join("|")),
        ArgKind::Pattern { source, .. } => format!("pattern {source}"),
        ArgKind::Int { min, max } => match (min, max) {
            (Some(min), Some(max)) => format!("int {min}..={max}"),
            (Some(min), None) => format!("int >= {min}"),
            (None, Some(max)) => format!("int <= {max}"),
            (None, None) => "int".to_owned(),
        },
        ArgKind::Flag => "flag".to_owned(),
        ArgKind::Text => "text".to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Clean,
    Problems,
}

struct Report {
    text: String,
    outcome: Outcome,
}

fn check(discovery: &Discovery) -> Report {
    let mut lines = Vec::new();
    let mut problem_count = 0;
    for problem in &discovery.problems {
        lines.push(problem.to_string());
        problem_count += 1;
    }
    for script in &discovery.scripts {
        if let Err(error) = load_header(script) {
            lines.push(error.to_string());
            problem_count += 1;
        }
    }
    for conflict in &discovery.conflicts {
        lines.push(format!(
            "conflict: {}: {}",
            conflict.name,
            joined_paths(&conflict.paths)
        ));
        problem_count += 1;
    }
    for unknown in &discovery.unknown {
        lines.push(format!(
            "warning: not a script (unknown extension): {}",
            unknown.path.display()
        ));
    }
    let outcome = if problem_count == 0 {
        let count = discovery.scripts.len();
        lines.push(format!("{count} {} OK", plural(count, "script", "scripts")));
        Outcome::Clean
    } else {
        lines.push(format!(
            "{problem_count} {}",
            plural(problem_count, "problem", "problems")
        ));
        Outcome::Problems
    };
    let mut text = lines.join("\n");
    text.push('\n');
    Report { text, outcome }
}

fn plural<'word>(count: usize, one: &'word str, many: &'word str) -> &'word str {
    if count == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_invocation(argv: &[&str]) -> Invocation {
        let cli = Cli::try_parse_from(argv).unwrap();
        let Action::Run(invocation) = cli.action else {
            panic!("expected run");
        };
        invocation
    }

    fn run_args(argv: &[&str]) -> (String, Vec<String>) {
        let invocation = run_invocation(argv);
        (invocation.name().to_owned(), invocation.args().to_vec())
    }

    #[test]
    fn trim_options_come_before_the_name() {
        let invocation = run_invocation(&["canif", "run", "--tail", "80", "--head=3", "x", "a"]);
        assert_eq!(invocation.head, NonZeroUsize::new(3));
        assert_eq!(invocation.tail, NonZeroUsize::new(80));
        assert_eq!(invocation.name(), "x");
        assert_eq!(invocation.args(), ["a"]);

        let invocation = run_invocation(&["canif", "run", "x", "--head", "3"]);
        assert_eq!(invocation.head, None);
        assert_eq!(invocation.args(), ["--head", "3"]);

        assert!(Cli::try_parse_from(["canif", "run", "--head", "3"]).is_err());

        for bad in ["0", "-1", "abc"] {
            assert!(Cli::try_parse_from(["canif", "run", "--head", bad, "x"]).is_err());
        }
        let cli = Cli::try_parse_from(["canif", "confirm", "--tail", "2", "x"]).unwrap();
        let Action::Confirm(invocation) = cli.action else {
            panic!("expected confirm");
        };
        assert_eq!(invocation.tail, NonZeroUsize::new(2));
    }

    #[test]
    fn run_passes_hyphen_args_verbatim() {
        let (name, args) = run_args(&["canif", "run", "x", "--filter", "foo", "--release"]);
        assert_eq!(name, "x");
        assert_eq!(args, ["--filter", "foo", "--release"]);
    }

    #[test]
    fn run_passes_help_and_double_dash_to_the_script() {
        let (_, args) = run_args(&["canif", "run", "x", "-5", "--help", "--", "-h"]);
        assert_eq!(args, ["-5", "--help", "--", "-h"]);
        let (_, args) = run_args(&["canif", "run", "x", "--help"]);
        assert_eq!(args, ["--help"]);
        let (_, args) = run_args(&["canif", "run", "x", "-h"]);
        assert_eq!(args, ["-h"]);
    }

    #[test]
    fn guide_example_parses() {
        let example = GUIDE
            .split("```")
            .nth(1)
            .expect("guide has a fenced example");
        let header = header::parse(example.trim_start()).unwrap();
        assert_eq!(
            usage::line("cargo/test", &header),
            "cargo/test <crate:core|cli> [--filter <filter>] [--jobs <jobs>=4] [--release]"
        );
    }

    #[test]
    fn confirm_parses_like_run() {
        let cli = Cli::try_parse_from(["canif", "confirm", "x", "--help"]).unwrap();
        let Action::Confirm(invocation) = cli.action else {
            panic!("expected confirm");
        };
        assert_eq!(invocation.name(), "x");
        assert_eq!(invocation.args(), ["--help"]);
    }

    #[test]
    fn wrong_subcommand_names_the_right_one() {
        assert_eq!(
            wrong_subcommand_text("deploy", Confirm::Required),
            "script `deploy` has `confirm: true`, run `canif confirm deploy`"
        );
        assert_eq!(
            wrong_subcommand_text("greet", Confirm::NotRequired),
            "script `greet` does not have `confirm: true`, run `canif run greet`"
        );
    }

    #[test]
    fn run_without_args() {
        let (name, args) = run_args(&["canif", "run", "x"]);
        assert_eq!(name, "x");
        assert!(args.is_empty());
    }
}
