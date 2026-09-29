use crate::header::{ArgKind, ArgSpec, Arity, Form, Header, Presence};

pub fn line(script_name: &str, header: &Header) -> String {
    let positional = header
        .args
        .iter()
        .filter(|(_, spec)| spec.form == Form::Positional)
        .map(|(name, spec)| positional_part(name, spec));
    let named = header
        .args
        .iter()
        .filter(|(_, spec)| spec.form == Form::Named)
        .map(|(name, spec)| named_part(name, spec));
    let mut line = script_name.to_owned();
    for part in positional.chain(named) {
        line.push(' ');
        line.push_str(&part);
    }
    line
}

/// Full invocation, through `canif auto` unless the script is risky.
pub fn command(script_name: &str, header: &Header) -> String {
    format!(
        "canif {} {}",
        header.risk.subcommand(),
        line(script_name, header)
    )
}

fn placeholder(name: &str, spec: &ArgSpec) -> String {
    match &spec.kind {
        ArgKind::Choices(choices) => format!("<{name}:{}>", choices.join("|")),
        _ => format!("<{name}>"),
    }
}

fn positional_part(name: &str, spec: &ArgSpec) -> String {
    let placeholder = placeholder(name, spec);
    match spec.arity {
        Arity::Many => format!("{placeholder}..."),
        Arity::Single => placeholder,
    }
}

fn named_part(name: &str, spec: &ArgSpec) -> String {
    if matches!(spec.kind, ArgKind::Flag) {
        return format!("[--{name}]");
    }
    let mut part = format!("--{name} {}", placeholder(name, spec));
    if let Some(default) = &spec.default {
        part.push('=');
        part.push_str(default);
    }
    match spec.presence {
        Presence::Required => part,
        Presence::Optional => format!("[{part}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header;

    fn header_with_args(args_yaml: &str) -> Header {
        let mut source = String::from("# ---\n# description: test\n# args:\n");
        for arg_line in args_yaml.lines() {
            source.push_str("#   ");
            source.push_str(arg_line);
            source.push('\n');
        }
        source.push_str("# ---\n");
        header::parse(&source).unwrap()
    }

    #[test]
    fn design_example() {
        let header = header_with_args(
            "crate:\n  choices: [core, cli]\nfilter:\n  pattern: ^[A-Za-z0-9_:]+$\n  optional: true\n  help: Test name filter\nrelease:\n  type: flag",
        );
        assert_eq!(
            line("cargo/test", &header),
            "cargo/test <crate:core|cli> [--filter <filter>] [--release]"
        );
    }

    #[test]
    fn many_arg() {
        let header = header_with_args("files:\n  many: true");
        assert_eq!(line("fmt/files", &header), "fmt/files <files>...");
        let header = header_with_args("mode:\n  choices: [a, b]\n  many: true");
        assert_eq!(line("x", &header), "x <mode:a|b>...");
    }

    #[test]
    fn named_args_follow_positional_in_header_order() {
        let header = header_with_args(
            "message:\n  named: true\nforce:\n  type: flag\nfiles:\n  many: true\ncount:\n  type: int\n  min: 1\n  optional: true",
        );
        assert_eq!(
            line("git/commit", &header),
            "git/commit <files>... --message <message> [--force] [--count <count>]"
        );
    }

    #[test]
    fn choices_on_optional_named() {
        let header = header_with_args("profile:\n  choices: [dev, release]\n  optional: true");
        assert_eq!(
            line("build", &header),
            "build [--profile <profile:dev|release>]"
        );
    }

    #[test]
    fn defaults_follow_placeholder() {
        let header = header_with_args(
            "count:\n  type: int\n  optional: true\n  default: 1\nprofile:\n  choices: [dev, release]\n  optional: true\n  default: dev",
        );
        assert_eq!(
            line("build", &header),
            "build [--count <count>=1] [--profile <profile:dev|release>=dev]"
        );
    }

    #[test]
    fn command_prefers_auto() {
        let header = header::parse("# ---\n# description: x\n# ---\n").unwrap();
        assert_eq!(command("hello", &header), "canif auto hello");
        let header = header::parse("# ---\n# description: x\n# risky: true\n# ---\n").unwrap();
        assert_eq!(command("deploy", &header), "canif run deploy");
    }

    #[test]
    fn no_args() {
        let header = header::parse("# ---\n# description: bare\n# ---\n").unwrap();
        assert_eq!(line("hello", &header), "hello");
    }
}
