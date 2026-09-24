use std::collections::HashMap;
use std::fmt;

use crate::header::{self, ArgKind, ArgSpec, Arity, Form, Header, Presence};
use crate::usage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    pub env: Vec<(String, String)>,
    pub argv: Vec<String>,
}

#[derive(Debug)]
pub struct BindError {
    pub arg: Option<String>,
    pub rule: Rule,
    pub usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rule {
    #[error("{} is not one of {}", shown(value), choices.join("|"))]
    NotInChoices { value: String, choices: Vec<String> },
    #[error("{} does not match pattern `{pattern}`", shown(value))]
    PatternMismatch { value: String, pattern: String },
    #[error("{} is not an integer", shown(value))]
    NotAnInt { value: String },
    #[error("{value} is out of range, {}", range_text(*min, *max))]
    OutOfRange {
        value: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    #[error(
        "{} starts with `-`, values may not (only int values may be negative)",
        shown(value)
    )]
    LeadingDash { value: String },
    #[error("{value:?} contains a newline, carriage return or NUL")]
    ForbiddenChar { value: String },
    #[error("missing required value")]
    Missing,
    #[error("unknown arg")]
    Unknown,
    #[error("given more than once")]
    Duplicate,
    #[error("positional arg, pass its value without `--name`")]
    PositionalByName,
    #[error("flag takes no value, pass it bare")]
    FlagWithValue,
    #[error("missing value after `--name`")]
    MissingValue,
    #[error("unexpected extra positional value {}", shown(value))]
    UnexpectedPositional { value: String },
    #[error("bare `--` is not accepted, there are no rest args")]
    BareDoubleDash,
}

impl fmt::Display for BindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(arg) = &self.arg {
            write!(formatter, "arg `{arg}`: ")?;
        }
        write!(formatter, "{}\nusage: {}", self.rule, self.usage)
    }
}

impl std::error::Error for BindError {}

enum Provided {
    Flag,
    Value(String),
}

pub fn bind(script_name: &str, header: &Header, raw: &[String]) -> Result<Bindings, BindError> {
    let fail = |arg: Option<&str>, rule: Rule| BindError {
        arg: arg.map(str::to_owned),
        rule,
        usage: usage::command(script_name, header),
    };

    let mut named: HashMap<&str, Provided> = HashMap::new();
    let mut positional_tokens: Vec<&String> = Vec::new();
    let mut tokens = raw.iter();
    while let Some(token) = tokens.next() {
        if token == "--" {
            return Err(fail(None, Rule::BareDoubleDash));
        }
        let Some(rest) = token.strip_prefix("--") else {
            positional_tokens.push(token);
            continue;
        };
        let (given_name, inline_value) = match rest.split_once('=') {
            Some((given_name, value)) => (given_name, Some(value)),
            None => (rest, None),
        };
        let Some((name, spec)) = header.args.get_key_value(given_name) else {
            return Err(fail(Some(given_name), Rule::Unknown));
        };
        let name = name.as_str();
        if spec.form == Form::Positional {
            return Err(fail(Some(name), Rule::PositionalByName));
        }
        if named.contains_key(name) {
            return Err(fail(Some(name), Rule::Duplicate));
        }
        let provided = if matches!(spec.kind, ArgKind::Flag) {
            if inline_value.is_some() {
                return Err(fail(Some(name), Rule::FlagWithValue));
            }
            Provided::Flag
        } else {
            let Some(value) = inline_value.or_else(|| tokens.next().map(String::as_str)) else {
                return Err(fail(Some(name), Rule::MissingValue));
            };
            Provided::Value(check_value(spec, value).map_err(|rule| fail(Some(name), rule))?)
        };
        named.insert(name, provided);
    }

    let mut positional_values: HashMap<&str, Vec<String>> = HashMap::new();
    let mut remaining = positional_tokens.into_iter();
    for (name, spec) in header
        .args
        .iter()
        .filter(|(_, spec)| spec.form == Form::Positional)
    {
        let values: Vec<&String> = match spec.arity {
            Arity::Many => remaining.by_ref().collect(),
            Arity::Single => remaining.next().into_iter().collect(),
        };
        if values.is_empty() {
            return Err(fail(Some(name), Rule::Missing));
        }
        let checked = values
            .into_iter()
            .map(|value| check_value(spec, value).map_err(|rule| fail(Some(name), rule)))
            .collect::<Result<Vec<_>, _>>()?;
        positional_values.insert(name, checked);
    }
    if let Some(extra) = remaining.next() {
        let value = extra.clone();
        return Err(fail(None, Rule::UnexpectedPositional { value }));
    }

    let mut bindings = Bindings {
        env: Vec::new(),
        argv: Vec::new(),
    };
    for (name, spec) in &header.args {
        let env_name = header::env_name(name);
        if let Some(values) = positional_values.remove(name.as_str()) {
            match spec.arity {
                Arity::Many => {
                    bindings.env.push((env_name, values.join("\n")));
                    bindings.argv = values;
                }
                Arity::Single => bindings
                    .env
                    .extend(values.into_iter().map(|value| (env_name.clone(), value))),
            }
            continue;
        }
        match named.remove(name.as_str()) {
            Some(Provided::Flag) => bindings.env.push((env_name, "1".to_owned())),
            Some(Provided::Value(value)) => bindings.env.push((env_name, value)),
            None if spec.presence == Presence::Required => {
                return Err(fail(Some(name), Rule::Missing));
            }
            None => {
                if let Some(default) = &spec.default {
                    bindings.env.push((env_name, default.clone()));
                }
            }
        }
    }
    Ok(bindings)
}

/// Validates `value` and returns the form passed to the script (canonical decimal for ints).
pub fn check_value(spec: &ArgSpec, value: &str) -> Result<String, Rule> {
    let owned = || value.to_owned();
    if value.contains(['\n', '\r', '\0']) {
        return Err(Rule::ForbiddenChar { value: owned() });
    }
    let is_int = matches!(spec.kind, ArgKind::Int { .. });
    if !is_int && value.starts_with('-') {
        return Err(Rule::LeadingDash { value: owned() });
    }
    match &spec.kind {
        ArgKind::Choices(choices) => {
            if !choices.iter().any(|choice| choice == value) {
                return Err(Rule::NotInChoices {
                    value: owned(),
                    choices: choices.clone(),
                });
            }
        }
        ArgKind::Pattern { regex, source } => {
            if !regex.is_match(value) {
                return Err(Rule::PatternMismatch {
                    value: owned(),
                    pattern: source.clone(),
                });
            }
        }
        ArgKind::Int { min, max } => {
            let Ok(number) = value.parse::<i64>() else {
                return Err(Rule::NotAnInt { value: owned() });
            };
            let below = min.is_some_and(|min| number < min);
            let above = max.is_some_and(|max| number > max);
            if below || above {
                return Err(Rule::OutOfRange {
                    value: number,
                    min: *min,
                    max: *max,
                });
            }
            return Ok(number.to_string());
        }
        ArgKind::Flag | ArgKind::Text => {}
    }
    Ok(owned())
}

fn shown(value: &str) -> String {
    if value.chars().any(char::is_control) {
        format!("{value:?}")
    } else {
        format!("`{value}`")
    }
}

fn range_text(min: Option<i64>, max: Option<i64>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("must be in {min}..={max}"),
        (Some(min), None) => format!("must be >= {min}"),
        (None, Some(max)) => format!("must be <= {max}"),
        (None, None) => "no bounds".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN_ARGS: &str = "crate:\n  choices: [core, cli]\nfilter:\n  pattern: ^[A-Za-z0-9_:]+$\n  optional: true\n  help: Test name filter\nrelease:\n  type: flag";

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

    fn tokens(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|token| (*token).to_owned()).collect()
    }

    fn bind_args(args_yaml: &str, raw: &[&str]) -> Result<Bindings, BindError> {
        bind("test/script", &header_with_args(args_yaml), &tokens(raw))
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn rule(args_yaml: &str, raw: &[&str]) -> (Option<String>, Rule) {
        let error = bind_args(args_yaml, raw).unwrap_err();
        (error.arg, error.rule)
    }

    #[test]
    fn design_example_happy_path() {
        let expected = env(&[
            ("CANIF_CRATE", "core"),
            ("CANIF_FILTER", "foo"),
            ("CANIF_RELEASE", "1"),
        ]);
        let bindings = bind_args(DESIGN_ARGS, &["core", "--filter", "foo", "--release"]).unwrap();
        assert_eq!(bindings.env, expected);
        assert!(bindings.argv.is_empty());
        let reordered = bind_args(DESIGN_ARGS, &["--release", "--filter=foo", "core"]).unwrap();
        assert_eq!(reordered.env, expected);
    }

    #[test]
    fn inline_named_value() {
        let bindings = bind_args(DESIGN_ARGS, &["--filter=foo", "core"]).unwrap();
        assert_eq!(
            bindings.env,
            env(&[("CANIF_CRATE", "core"), ("CANIF_FILTER", "foo")])
        );
    }

    #[test]
    fn absent_optional_leaves_no_env() {
        let bindings = bind_args(DESIGN_ARGS, &["cli"]).unwrap();
        assert_eq!(bindings.env, env(&[("CANIF_CRATE", "cli")]));
    }

    #[test]
    fn default_exported_when_omitted_and_overridden_when_given() {
        let args =
            "count:\n  type: int\n  optional: true\n  default: '05'\nfilter:\n  optional: true";
        let omitted = bind_args(args, &[]).unwrap();
        assert_eq!(omitted.env, env(&[("CANIF_COUNT", "5")]));
        let given = bind_args(args, &["--count=2"]).unwrap();
        assert_eq!(given.env, env(&[("CANIF_COUNT", "2")]));
    }

    #[test]
    fn many_values() {
        let args = "files:\n  many: true\nforce:\n  type: flag";
        let one = bind_args(args, &["a.rs"]).unwrap();
        assert_eq!(one.argv, tokens(&["a.rs"]));
        assert_eq!(one.env, env(&[("CANIF_FILES", "a.rs")]));
        let three = bind_args(args, &["a.rs", "--force", "b.rs", "c.rs"]).unwrap();
        assert_eq!(three.argv, tokens(&["a.rs", "b.rs", "c.rs"]));
        assert_eq!(
            three.env,
            env(&[("CANIF_FILES", "a.rs\nb.rs\nc.rs"), ("CANIF_FORCE", "1")])
        );
    }

    #[test]
    fn many_after_single_positional() {
        let bindings = bind_args("mode: {}\nfiles:\n  many: true", &["fix", "a", "b"]).unwrap();
        assert_eq!(bindings.argv, tokens(&["a", "b"]));
        assert_eq!(
            bindings.env,
            env(&[("CANIF_MODE", "fix"), ("CANIF_FILES", "a\nb")])
        );
    }

    #[test]
    fn many_missing() {
        assert_eq!(
            rule("files:\n  many: true", &[]),
            (Some("files".to_owned()), Rule::Missing)
        );
    }

    #[test]
    fn missing_positional() {
        assert_eq!(
            rule(DESIGN_ARGS, &["--release"]),
            (Some("crate".to_owned()), Rule::Missing)
        );
    }

    #[test]
    fn missing_required_named() {
        assert_eq!(
            rule("message:\n  named: true", &[]),
            (Some("message".to_owned()), Rule::Missing)
        );
        let bindings = bind_args("message:\n  named: true", &["--message", "hi"]).unwrap();
        assert_eq!(bindings.env, env(&[("CANIF_MESSAGE", "hi")]));
    }

    #[test]
    fn negative_int_accepted() {
        let args = "offset:\n  type: int\nshift:\n  type: int\n  optional: true";
        let bindings = bind_args(args, &["-5", "--shift", "-3"]).unwrap();
        assert_eq!(
            bindings.env,
            env(&[("CANIF_OFFSET", "-5"), ("CANIF_SHIFT", "-3")])
        );
        let inline = bind_args(args, &["--shift=-3", "-5"]).unwrap();
        assert_eq!(inline.env, bindings.env);
    }

    #[test]
    fn int_passed_in_canonical_form() {
        let args = "count:\n  type: int\nshift:\n  type: int\n  optional: true";
        let bindings = bind_args(args, &["+5", "--shift=-007"]).unwrap();
        assert_eq!(
            bindings.env,
            env(&[("CANIF_COUNT", "5"), ("CANIF_SHIFT", "-7")])
        );
        let many = bind_args("ids:\n  type: int\n  many: true", &["0008", "+1"]).unwrap();
        assert_eq!(many.argv, tokens(&["8", "1"]));
    }

    #[test]
    fn int_out_of_range() {
        let args = "count:\n  type: int\n  min: 1\n  max: 5";
        assert_eq!(
            rule(args, &["6"]).1,
            Rule::OutOfRange {
                value: 6,
                min: Some(1),
                max: Some(5)
            }
        );
        assert!(matches!(rule(args, &["0"]).1, Rule::OutOfRange { .. }));
        assert!(bind_args(args, &["5"]).is_ok());
    }

    #[test]
    fn int_garbage() {
        for value in ["abc", "1.5", "", "99999999999999999999"] {
            assert_eq!(
                rule("count:\n  type: int", &[value]).1,
                Rule::NotAnInt {
                    value: value.to_owned()
                },
                "{value}"
            );
        }
    }

    #[test]
    fn not_in_choices() {
        assert_eq!(
            rule(DESIGN_ARGS, &["nope"]),
            (
                Some("crate".to_owned()),
                Rule::NotInChoices {
                    value: "nope".to_owned(),
                    choices: tokens(&["core", "cli"])
                }
            )
        );
    }

    #[test]
    fn pattern_mismatch_shows_source_pattern() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--filter", "a b"]).1,
            Rule::PatternMismatch {
                value: "a b".to_owned(),
                pattern: "^[A-Za-z0-9_:]+$".to_owned()
            }
        );
    }

    #[test]
    fn leading_dash_rejected() {
        for (args, raw) in [
            ("name: {}", vec!["-x"]),
            ("mode:\n  choices: [a, b]", vec!["-a"]),
            ("id:\n  pattern: '-?[a-z]+'", vec!["-abc"]),
            ("name:\n  optional: true", vec!["--name=-x"]),
        ] {
            assert!(
                matches!(rule(args, &raw).1, Rule::LeadingDash { .. }),
                "{args} {raw:?}"
            );
        }
    }

    #[test]
    fn named_value_taken_from_next_token_even_if_dashed() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--filter", "--release"]),
            (
                Some("filter".to_owned()),
                Rule::LeadingDash {
                    value: "--release".to_owned()
                }
            )
        );
    }

    #[test]
    fn forbidden_chars_rejected() {
        for value in ["a\nb", "a\rb", "a\0b"] {
            assert!(
                matches!(rule("name: {}", &[value]).1, Rule::ForbiddenChar { .. }),
                "{value:?}"
            );
            assert!(
                matches!(
                    rule("count:\n  type: int", &[value]).1,
                    Rule::ForbiddenChar { .. }
                ),
                "{value:?}"
            );
        }
        assert!(matches!(
            rule("files:\n  many: true", &["ok", "bad\nvalue"]).1,
            Rule::ForbiddenChar { .. }
        ));
    }

    #[test]
    fn unknown_named() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--verbose"]),
            (Some("verbose".to_owned()), Rule::Unknown)
        );
    }

    #[test]
    fn positional_by_name() {
        assert_eq!(
            rule(DESIGN_ARGS, &["--crate", "core"]),
            (Some("crate".to_owned()), Rule::PositionalByName)
        );
    }

    #[test]
    fn duplicate_named() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--filter", "a", "--filter=b"]),
            (Some("filter".to_owned()), Rule::Duplicate)
        );
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--release", "--release"]),
            (Some("release".to_owned()), Rule::Duplicate)
        );
    }

    #[test]
    fn flag_with_value() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--release=1"]),
            (Some("release".to_owned()), Rule::FlagWithValue)
        );
    }

    #[test]
    fn missing_named_value() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--filter"]),
            (Some("filter".to_owned()), Rule::MissingValue)
        );
    }

    #[test]
    fn unexpected_positional() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "cli"]),
            (
                None,
                Rule::UnexpectedPositional {
                    value: "cli".to_owned()
                }
            )
        );
    }

    #[test]
    fn bare_double_dash() {
        assert_eq!(
            rule(DESIGN_ARGS, &["core", "--", "x"]),
            (None, Rule::BareDoubleDash)
        );
    }

    #[test]
    fn display_has_arg_rule_and_usage() {
        let error = bind(
            "cargo/test",
            &header_with_args(DESIGN_ARGS),
            &tokens(&["nope"]),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "arg `crate`: `nope` is not one of core|cli\nusage: canif run cargo/test <crate:core|cli> [--filter <filter>] [--release]"
        );
    }

    #[test]
    fn display_escapes_control_chars() {
        let error = bind_args(DESIGN_ARGS, &["core", "--filter", "a\0b"]).unwrap_err();
        let message = error.to_string();
        assert!(message.starts_with("arg `filter`: \"a\\0b\""), "{message}");
        assert!(!message.contains('\0'));
        assert_eq!(message.lines().count(), 2);
    }
}
