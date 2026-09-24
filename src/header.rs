use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;

const COMMENT_PREFIXES: [&str; 2] = ["#", "//"];
const FENCE: &str = "---";

#[derive(Debug, Clone)]
pub struct Header {
    pub description: String,
    pub args: IndexMap<String, ArgSpec>,
}

#[derive(Debug, Clone)]
pub struct ArgSpec {
    pub kind: ArgKind,
    pub presence: Presence,
    pub form: Form,
    pub arity: Arity,
    pub help: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ArgKind {
    Choices(Vec<String>),
    Pattern(Regex),
    Int { min: Option<i64>, max: Option<i64> },
    Flag,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Required,
    Optional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Positional,
    Named,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Single,
    Many,
}

#[derive(Debug, thiserror::Error)]
pub enum HeaderError {
    #[error(
        "missing header: expected a `# ---` or `// ---` fence on the first line (after an optional shebang)"
    )]
    Missing,
    #[error("unterminated header: no closing `---` fence")]
    Unterminated,
    #[error("line {line}: expected a comment line inside the header")]
    NotComment { line: usize },
    #[error("invalid header YAML: {0}")]
    Yaml(#[from] serde_saphyr::Error),
    #[error(transparent)]
    Spec(#[from] SpecError),
}

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("arg `{arg}`: name must match [a-z][a-z0-9-]*")]
    InvalidName { arg: String },
    #[error("arg `{arg}`: at most one of `choices`, `pattern`, `type` may be given")]
    ConflictingKinds { arg: String },
    #[error("arg `{arg}`: `min` / `max` require `type: int`")]
    MinMaxWithoutInt { arg: String },
    #[error("arg `{arg}`: `min` ({min}) is greater than `max` ({max})")]
    MinAboveMax { arg: String, min: i64, max: i64 },
    #[error("arg `{arg}`: `choices` is empty")]
    EmptyChoices { arg: String },
    #[error("arg `{arg}`: choice `{choice:?}` can never be passed (leading `-`, newline or NUL)")]
    UnpassableChoice { arg: String, choice: String },
    #[error("arg `{arg}`: invalid pattern: {error}")]
    InvalidPattern { arg: String, error: regex::Error },
    #[error("arg `{arg}`: only one `many` arg is allowed, `{first}` already is")]
    MultipleMany { arg: String, first: String },
    #[error("arg `{arg}`: `many` arg must be the last positional, but `{next}` follows it")]
    ManyNotLast { arg: String, next: String },
    #[error("arg `{arg}`: `many` cannot be `optional`")]
    ManyOptional { arg: String },
    #[error("arg `{arg}`: `many` cannot be `named`")]
    ManyNamed { arg: String },
    #[error("arg `{arg}`: `many` cannot be a flag")]
    ManyFlag { arg: String },
    #[error("arg `{arg}`: flags are always optional, remove `optional`")]
    FlagOptional { arg: String },
    #[error("arg `{arg}`: flags are always named, remove `named`")]
    FlagNamed { arg: String },
    #[error("arg `{arg}`: optional args are always named, remove `named`")]
    NamedOptional { arg: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeader {
    description: String,
    args: Option<IndexMap<String, RawArg>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArg {
    choices: Option<Vec<String>>,
    pattern: Option<String>,
    #[serde(rename = "type")]
    value_type: Option<RawType>,
    min: Option<i64>,
    max: Option<i64>,
    #[serde(default)]
    optional: bool,
    help: Option<String>,
    #[serde(default)]
    named: bool,
    #[serde(default)]
    many: bool,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum RawType {
    Int,
    Flag,
}

pub fn parse(source: &str) -> Result<Header, HeaderError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let yaml = extract_block(source)?;
    let raw: RawHeader = serde_saphyr::from_str(&yaml)?;
    let mut args = IndexMap::new();
    for (name, raw_arg) in raw.args.unwrap_or_default() {
        let spec = build_spec(&name, raw_arg)?;
        args.insert(name, spec);
    }
    validate_many(&args)?;
    Ok(Header {
        description: raw.description,
        args,
    })
}

pub fn env_name(arg_name: &str) -> String {
    format!("CANIF_{}", arg_name.to_ascii_uppercase().replace('-', "_"))
}

fn extract_block(source: &str) -> Result<String, HeaderError> {
    let mut lines = source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line));
    let mut first = lines.next();
    if first.is_some_and(|(_, line)| line.starts_with("#!")) {
        first = lines.next();
    }
    let Some((_, fence_line)) = first else {
        return Err(HeaderError::Missing);
    };
    let Some(prefix) = COMMENT_PREFIXES
        .into_iter()
        .find(|prefix| strip_comment(fence_line, prefix).is_some_and(is_fence))
    else {
        return Err(HeaderError::Missing);
    };
    let mut yaml = String::new();
    for (line_number, line) in lines {
        let Some(content) = strip_comment(line, prefix) else {
            return Err(HeaderError::NotComment { line: line_number });
        };
        if is_fence(content) {
            return Ok(yaml);
        }
        yaml.push_str(content);
        yaml.push('\n');
    }
    Err(HeaderError::Unterminated)
}

fn strip_comment<'line>(line: &'line str, prefix: &str) -> Option<&'line str> {
    let rest = line.strip_prefix(prefix)?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn is_fence(content: &str) -> bool {
    content.trim_end() == FENCE
}

fn is_valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn build_spec(name: &str, raw: RawArg) -> Result<ArgSpec, SpecError> {
    let arg = name.to_owned();
    if !is_valid_name(name) {
        return Err(SpecError::InvalidName { arg });
    }
    let kinds_given = [
        raw.choices.is_some(),
        raw.pattern.is_some(),
        raw.value_type.is_some(),
    ]
    .into_iter()
    .filter(|given| *given)
    .count();
    if kinds_given > 1 {
        return Err(SpecError::ConflictingKinds { arg });
    }
    if (raw.min.is_some() || raw.max.is_some()) && raw.value_type != Some(RawType::Int) {
        return Err(SpecError::MinMaxWithoutInt { arg });
    }
    let kind = match (raw.choices, raw.pattern, raw.value_type) {
        (Some(choices), _, _) => {
            if choices.is_empty() {
                return Err(SpecError::EmptyChoices { arg });
            }
            if let Some(choice) = choices.iter().find(|choice| !is_passable_choice(choice)) {
                let choice = choice.clone();
                return Err(SpecError::UnpassableChoice { arg, choice });
            }
            ArgKind::Choices(choices)
        }
        (_, Some(pattern), _) => ArgKind::Pattern(compile_pattern(name, &pattern)?),
        (_, _, Some(RawType::Int)) => {
            if let (Some(min), Some(max)) = (raw.min, raw.max)
                && min > max
            {
                return Err(SpecError::MinAboveMax { arg, min, max });
            }
            ArgKind::Int {
                min: raw.min,
                max: raw.max,
            }
        }
        (_, _, Some(RawType::Flag)) => ArgKind::Flag,
        (None, None, None) => ArgKind::Text,
    };
    let is_flag = matches!(kind, ArgKind::Flag);
    if raw.many {
        if raw.optional {
            return Err(SpecError::ManyOptional { arg });
        }
        if raw.named {
            return Err(SpecError::ManyNamed { arg });
        }
        if is_flag {
            return Err(SpecError::ManyFlag { arg });
        }
    }
    if is_flag && raw.optional {
        return Err(SpecError::FlagOptional { arg });
    }
    if is_flag && raw.named {
        return Err(SpecError::FlagNamed { arg });
    }
    if raw.named && raw.optional {
        return Err(SpecError::NamedOptional { arg });
    }
    Ok(ArgSpec {
        kind,
        presence: if raw.optional || is_flag {
            Presence::Optional
        } else {
            Presence::Required
        },
        form: if raw.optional || raw.named || is_flag {
            Form::Named
        } else {
            Form::Positional
        },
        arity: if raw.many { Arity::Many } else { Arity::Single },
        help: raw.help,
    })
}

fn is_passable_choice(choice: &str) -> bool {
    !choice.starts_with('-') && !choice.contains(['\n', '\r', '\0'])
}

fn compile_pattern(name: &str, pattern: &str) -> Result<Regex, SpecError> {
    let invalid = |error| SpecError::InvalidPattern {
        arg: name.to_owned(),
        error,
    };
    // Compiling alone first rejects unbalanced groups like `a)|(b` that would escape the anchors.
    Regex::new(pattern).map_err(invalid)?;
    Regex::new(&format!("^(?:{pattern})$")).map_err(invalid)
}

fn validate_many(args: &IndexMap<String, ArgSpec>) -> Result<(), SpecError> {
    let mut many_arg: Option<&str> = None;
    for (name, spec) in args {
        if spec.arity == Arity::Many {
            if let Some(first) = many_arg {
                return Err(SpecError::MultipleMany {
                    arg: name.clone(),
                    first: first.to_owned(),
                });
            }
            many_arg = Some(name);
        } else if let Some(many_name) = many_arg
            && spec.form == Form::Positional
        {
            return Err(SpecError::ManyNotLast {
                arg: many_name.to_owned(),
                next: name.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN_EXAMPLE: &str = "#!/usr/bin/env bash
# ---
# description: Run tests of one crate
# args:
#   crate:
#     choices: [core, cli]
#   filter:
#     pattern: ^[A-Za-z0-9_:]+$
#     optional: true
#     help: Test name filter
#   release:
#     type: flag
# ---
cargo test -p \"$CANIF_CRATE\" ${CANIF_RELEASE:+--release} -- \"${CANIF_FILTER:-}\"
";

    fn header_with_args(args_yaml: &str) -> String {
        let mut source = String::from("# ---\n# description: test\n# args:\n");
        for line in args_yaml.lines() {
            source.push_str("#   ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("# ---\n");
        source
    }

    fn spec_error(args_yaml: &str) -> SpecError {
        match parse(&header_with_args(args_yaml)) {
            Err(HeaderError::Spec(error)) => error,
            other => panic!("expected spec error, got {other:?}"),
        }
    }

    #[test]
    fn design_example_parses() {
        let header = parse(DESIGN_EXAMPLE).unwrap();
        assert_eq!(header.description, "Run tests of one crate");
        let names: Vec<&str> = header.args.keys().map(String::as_str).collect();
        assert_eq!(names, ["crate", "filter", "release"]);

        let crate_arg = &header.args["crate"];
        assert!(
            matches!(&crate_arg.kind, ArgKind::Choices(choices) if choices == &["core", "cli"])
        );
        assert_eq!(crate_arg.presence, Presence::Required);
        assert_eq!(crate_arg.form, Form::Positional);
        assert_eq!(crate_arg.arity, Arity::Single);

        let filter = &header.args["filter"];
        let ArgKind::Pattern(regex) = &filter.kind else {
            panic!("filter should be a pattern");
        };
        assert!(regex.is_match("foo::bar"));
        assert!(!regex.is_match("foo bar"));
        assert_eq!(filter.presence, Presence::Optional);
        assert_eq!(filter.form, Form::Named);
        assert_eq!(filter.help.as_deref(), Some("Test name filter"));

        let release = &header.args["release"];
        assert!(matches!(release.kind, ArgKind::Flag));
        assert_eq!(release.presence, Presence::Optional);
        assert_eq!(release.form, Form::Named);
    }

    #[test]
    fn slash_prefix() {
        let source = "// ---\n// description: js\n// args:\n//   count:\n//     type: int\n//     min: 1\n//     max: 5\n// ---\nconsole.log(1);\n";
        let header = parse(source).unwrap();
        assert_eq!(header.description, "js");
        assert!(matches!(
            header.args["count"].kind,
            ArgKind::Int {
                min: Some(1),
                max: Some(5)
            }
        ));
    }

    #[test]
    fn without_shebang_and_without_args() {
        let header = parse("# ---\n# description: bare\n# ---\necho hi\n").unwrap();
        assert_eq!(header.description, "bare");
        assert!(header.args.is_empty());
    }

    #[test]
    fn crlf_line_endings() {
        let source = DESIGN_EXAMPLE.replace('\n', "\r\n");
        let header = parse(&source).unwrap();
        assert_eq!(header.description, "Run tests of one crate");
        assert_eq!(header.args.len(), 3);
    }

    #[test]
    fn free_text_and_named_and_many() {
        let header = parse(&header_with_args(
            "message:\n  named: true\nfiles:\n  many: true\nverbose:\n  type: flag",
        ))
        .unwrap();
        let message = &header.args["message"];
        assert!(matches!(message.kind, ArgKind::Text));
        assert_eq!(message.presence, Presence::Required);
        assert_eq!(message.form, Form::Named);
        let files = &header.args["files"];
        assert_eq!(files.arity, Arity::Many);
        assert_eq!(files.form, Form::Positional);
    }

    #[test]
    fn missing_header() {
        assert!(matches!(parse(""), Err(HeaderError::Missing)));
        assert!(matches!(
            parse("#!/bin/sh\necho hi\n"),
            Err(HeaderError::Missing)
        ));
        assert!(matches!(
            parse("#!/bin/sh\n\n# ---\n# description: x\n# ---\n"),
            Err(HeaderError::Missing)
        ));
    }

    #[test]
    fn unterminated_header() {
        assert!(matches!(
            parse("# ---\n# description: x\n"),
            Err(HeaderError::Unterminated)
        ));
    }

    #[test]
    fn non_comment_line_inside_header() {
        assert!(matches!(
            parse("# ---\n# description: x\necho hi\n# ---\n"),
            Err(HeaderError::NotComment { line: 3 })
        ));
        assert!(matches!(
            parse("# ---\n// description: x\n# ---\n"),
            Err(HeaderError::NotComment { line: 2 })
        ));
    }

    #[test]
    fn unknown_top_level_key() {
        assert!(matches!(
            parse("# ---\n# description: x\n# tags: [a]\n# ---\n"),
            Err(HeaderError::Yaml(_))
        ));
    }

    #[test]
    fn unknown_arg_key() {
        assert!(matches!(
            parse(&header_with_args("name:\n  default: x")),
            Err(HeaderError::Yaml(_))
        ));
    }

    #[test]
    fn missing_description() {
        assert!(matches!(
            parse("# ---\n# args: {}\n# ---\n"),
            Err(HeaderError::Yaml(_))
        ));
    }

    #[test]
    fn duplicate_arg_name() {
        let result = parse(&header_with_args(
            "name:\n  optional: true\nname:\n  type: flag",
        ));
        let Err(HeaderError::Yaml(error)) = result else {
            panic!("expected YAML error, got {result:?}");
        };
        assert!(
            error.to_string().to_lowercase().contains("duplicate"),
            "{error}"
        );
    }

    #[test]
    fn invalid_names() {
        for name in ["Name", "1st", "-x", "snake_case", "a.b"] {
            assert!(
                matches!(
                    spec_error(&format!("{name}: {{}}")),
                    SpecError::InvalidName { .. }
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn env_names() {
        assert_eq!(env_name("dry-run"), "CANIF_DRY_RUN");
        assert_eq!(env_name("crate"), "CANIF_CRATE");
        assert_eq!(env_name("a1-b2"), "CANIF_A1_B2");
    }

    #[test]
    fn conflicting_kinds() {
        for arg in [
            "x:\n  choices: [a]\n  pattern: a",
            "x:\n  choices: [a]\n  type: int",
            "x:\n  pattern: a\n  type: flag",
        ] {
            assert!(
                matches!(spec_error(arg), SpecError::ConflictingKinds { .. }),
                "{arg}"
            );
        }
    }

    #[test]
    fn min_max_without_int() {
        assert!(matches!(
            spec_error("x:\n  min: 1"),
            SpecError::MinMaxWithoutInt { .. }
        ));
        assert!(matches!(
            spec_error("x:\n  type: flag\n  max: 1"),
            SpecError::MinMaxWithoutInt { .. }
        ));
    }

    #[test]
    fn min_above_max() {
        assert!(matches!(
            spec_error("x:\n  type: int\n  min: 5\n  max: 1"),
            SpecError::MinAboveMax { min: 5, max: 1, .. }
        ));
    }

    #[test]
    fn unknown_type() {
        assert!(matches!(
            parse(&header_with_args("x:\n  type: path")),
            Err(HeaderError::Yaml(_))
        ));
    }

    #[test]
    fn empty_choices() {
        assert!(matches!(
            spec_error("x:\n  choices: []"),
            SpecError::EmptyChoices { .. }
        ));
    }

    #[test]
    fn unpassable_choices() {
        for arg in ["x:\n  choices: [a, -b]", "x:\n  choices: [\"a\\nb\"]"] {
            assert!(
                matches!(spec_error(arg), SpecError::UnpassableChoice { .. }),
                "{arg}"
            );
        }
    }

    #[test]
    fn utf8_bom_is_ignored() {
        let header = parse("\u{feff}# ---\n# description: bom\n# ---\n").unwrap();
        assert_eq!(header.description, "bom");
    }

    #[test]
    fn invalid_regex() {
        assert!(matches!(
            spec_error("x:\n  pattern: '[a-'"),
            SpecError::InvalidPattern { .. }
        ));
    }

    #[test]
    fn unbalanced_group_cannot_escape_anchors() {
        assert!(matches!(
            spec_error("x:\n  pattern: 'a)|(b'"),
            SpecError::InvalidPattern { .. }
        ));
    }

    #[test]
    fn pattern_is_anchored() {
        let header = parse(&header_with_args("x:\n  pattern: a+")).unwrap();
        let ArgKind::Pattern(regex) = &header.args["x"].kind else {
            panic!("x should be a pattern");
        };
        assert!(regex.is_match("aaa"));
        assert!(!regex.is_match("baa"));
        assert!(!regex.is_match("aab"));
    }

    #[test]
    fn alternation_is_anchored_as_a_whole() {
        let header = parse(&header_with_args("x:\n  pattern: a|b")).unwrap();
        let ArgKind::Pattern(regex) = &header.args["x"].kind else {
            panic!("x should be a pattern");
        };
        assert!(regex.is_match("a"));
        assert!(!regex.is_match("ab"));
        assert!(!regex.is_match("ba"));
    }

    #[test]
    fn multiple_many() {
        assert!(matches!(
            spec_error("a:\n  many: true\nb:\n  many: true"),
            SpecError::MultipleMany { .. }
        ));
    }

    #[test]
    fn many_not_last_positional() {
        let SpecError::ManyNotLast { arg, next } = spec_error("a:\n  many: true\nb: {}") else {
            panic!("expected ManyNotLast");
        };
        assert_eq!((arg.as_str(), next.as_str()), ("a", "b"));
    }

    #[test]
    fn many_followed_by_named_args_is_valid() {
        let header = parse(&header_with_args(
            "files:\n  many: true\nmode:\n  optional: true\nforce:\n  type: flag\nlabel:\n  named: true",
        ))
        .unwrap();
        assert_eq!(header.args["files"].arity, Arity::Many);
    }

    #[test]
    fn many_combinations() {
        assert!(matches!(
            spec_error("a:\n  many: true\n  optional: true"),
            SpecError::ManyOptional { .. }
        ));
        assert!(matches!(
            spec_error("a:\n  many: true\n  named: true"),
            SpecError::ManyNamed { .. }
        ));
        assert!(matches!(
            spec_error("a:\n  many: true\n  type: flag"),
            SpecError::ManyFlag { .. }
        ));
    }

    #[test]
    fn flag_combinations() {
        assert!(matches!(
            spec_error("a:\n  type: flag\n  optional: true"),
            SpecError::FlagOptional { .. }
        ));
        assert!(matches!(
            spec_error("a:\n  type: flag\n  named: true"),
            SpecError::FlagNamed { .. }
        ));
    }

    #[test]
    fn named_with_optional() {
        assert!(matches!(
            spec_error("a:\n  named: true\n  optional: true"),
            SpecError::NamedOptional { .. }
        ));
    }
}
