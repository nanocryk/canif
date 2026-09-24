use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::binding::{self, BindError};
use crate::discovery::{self, Discovery, DiscoveryError, Script};
use crate::exec::{self, ExecError};
use crate::header::{self, ArgKind, ArgSpec, Arity, Header, HeaderError, Presence};
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
    Run {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Validate every script header and report conflicts.
    Check,
}

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
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error(transparent)]
    Exec(#[from] ExecError),
}

/// Runs the parsed command and returns the process exit code.
pub fn execute(cli: Cli) -> Result<i32, CliError> {
    let cwd = std::env::current_dir().map_err(CliError::CurrentDir)?;
    let home = std::env::home_dir();
    let discovery = discovery::discover(&cwd, home.as_deref())?;
    match cli.action {
        Action::List { terms } => {
            print!("{}", list(&discovery, &terms));
            Ok(0)
        }
        Action::Show { name } => {
            let script = find(&discovery, &name)?;
            print!("{}", show(script, &load_header(script)?));
            Ok(0)
        }
        Action::Run { name, args } => {
            let script = find(&discovery, &name)?;
            let header = load_header(script)?;
            let bindings = binding::bind(&script.name, &header, &args)?;
            let origin = &discovery.origins[script.origin];
            Ok(exec::run(script, origin, &discovery.config, &bindings)?)
        }
        Action::Check => {
            let report = check(&discovery);
            print!("{}", report.text);
            Ok(match report.outcome {
                Outcome::Clean => 0,
                Outcome::Problems => 1,
            })
        }
    }
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
                    "  {}  # {}",
                    usage::line(&script.name, header),
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
        format!("usage: canif run {}", usage::line(&script.name, header)),
        format!("description: {}", header.description),
        format!("source: {}", script.path.display()),
    ];
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

    fn run_args(argv: &[&str]) -> (String, Vec<String>) {
        let cli = Cli::try_parse_from(argv).unwrap();
        let Action::Run { name, args } = cli.action else {
            panic!("expected run");
        };
        (name, args)
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
    }

    #[test]
    fn run_without_args() {
        let (name, args) = run_args(&["canif", "run", "x"]);
        assert_eq!(name, "x");
        assert!(args.is_empty());
    }
}
