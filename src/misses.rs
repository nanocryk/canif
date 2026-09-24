use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::discovery;

pub const FILE_NAME: &str = ".misses.log";
const KEPT_LINES: usize = 1000;
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    pub timestamp: String,
    pub cwd: String,
    pub name: String,
    pub reason: String,
    pub argv: Vec<String>,
}

impl Miss {
    /// Tab-separated fields with one field per argv value, newline-terminated.
    pub fn to_line(&self) -> String {
        let mut fields = vec![
            escape(&self.timestamp),
            escape(&self.cwd),
            escape(&self.name),
            escape(&self.reason),
        ];
        fields.extend(self.argv.iter().map(|value| escape(value)));
        let mut line = fields.join("\t");
        line.push('\n');
        line
    }

    /// `line` without its newline; `None` when malformed.
    pub fn parse(line: &str) -> Option<Miss> {
        let mut fields = line.split('\t').map(unescape);
        Some(Miss {
            timestamp: fields.next()??,
            cwd: fields.next()??,
            name: fields.next()??,
            reason: fields.next()??,
            argv: fields.collect::<Option<_>>()?,
        })
    }
}

pub fn log_path(home: &Path) -> PathBuf {
    home.join(discovery::DIR_NAME).join(FILE_NAME)
}

/// Appends a miss for the current time and cwd. A failure only prints one warning.
pub fn record(home: Option<&Path>, name: &str, argv: &[String], reason: String) {
    let Some(home) = home else {
        return;
    };
    let miss = Miss {
        timestamp: now_timestamp(),
        cwd: std::env::current_dir()
            .map(|cwd| cwd.to_string_lossy().into_owned())
            .unwrap_or_default(),
        name: name.to_owned(),
        reason,
        argv: argv.to_vec(),
    };
    let path = log_path(home);
    if let Err(error) = append(&path, miss.to_line().as_bytes(), KEPT_LINES) {
        eprintln!("canif: cannot write miss log {}: {error}", path.display());
    }
}

fn append(path: &Path, line: &[u8], kept: usize) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(line)?;
    cap(path, kept)
}

/// Rewrites the file with its last `kept` lines when it holds more.
fn cap(path: &Path, kept: usize) -> io::Result<()> {
    let content = fs::read(path)?;
    let lines: Vec<&[u8]> = content.split_inclusive(|byte| *byte == b'\n').collect();
    if lines.len() <= kept {
        return Ok(());
    }
    let temp = path.with_file_name(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    let result = fs::write(&temp, lines[lines.len() - kept..].concat())
        .and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Per script name, the total then each distinct reason, most frequent first, ties by text.
pub fn summary(content: &str) -> String {
    let mut groups: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for miss in content.lines().filter_map(Miss::parse) {
        *groups
            .entry(miss.name)
            .or_default()
            .entry(miss.reason)
            .or_default() += 1;
    }
    if groups.is_empty() {
        return "no misses recorded\n".to_owned();
    }
    let totals = groups
        .iter()
        .map(|(name, reasons)| (name.clone(), reasons.values().sum()))
        .collect();
    let mut output = String::new();
    for (total, name) in by_count(totals) {
        output.push_str(&format!("{total}  {name}\n"));
        for (count, reason) in by_count(groups.remove(&name).unwrap_or_default()) {
            output.push_str(&format!("     {count}  {reason}\n"));
        }
    }
    output
}

/// Highest count first, ties by key.
fn by_count(counts: BTreeMap<String, usize>) -> Vec<(usize, String)> {
    let mut sorted: Vec<(usize, String)> = counts
        .into_iter()
        .map(|(key, count)| (count, key))
        .collect();
    sorted.sort_by_key(|(count, _)| Reverse(*count));
    sorted
}

fn escape(field: &str) -> String {
    let mut escaped = String::with_capacity(field.len());
    for character in field.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn unescape(field: &str) -> Option<String> {
    let mut unescaped = String::with_capacity(field.len());
    let mut characters = field.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        unescaped.push(match characters.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(unescaped)
}

fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format_timestamp(seconds)
}

/// ISO-8601 UTC with second precision, e.g. `2026-09-24T13:05:09Z`.
fn format_timestamp(unix_seconds: u64) -> String {
    let (year, month, day) = civil_from_days(unix_seconds / SECONDS_PER_DAY);
    let second_of_day = unix_seconds % SECONDS_PER_DAY;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days` for days since 1970-01-01, with years starting in March.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miss(name: &str, reason: &str) -> Miss {
        Miss {
            timestamp: "2026-09-24T13:05:09Z".to_owned(),
            cwd: "/work".to_owned(),
            name: name.to_owned(),
            reason: reason.to_owned(),
            argv: Vec::new(),
        }
    }

    #[test]
    fn timestamps() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_timestamp(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(format_timestamp(1_709_251_200), "2024-03-01T00:00:00Z");
        assert_eq!(format_timestamp(1_790_255_109), "2026-09-24T13:05:09Z");
        assert_eq!(format_timestamp(4_107_542_399), "2100-02-28T23:59:59Z");
        assert_eq!(format_timestamp(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    #[test]
    fn lines_round_trip() {
        let odd = Miss {
            timestamp: "t".to_owned(),
            cwd: "C:\\dir\twith\ttabs".to_owned(),
            name: "a\\tb".to_owned(),
            reason: "arg `x`: \"\\n\" bad".to_owned(),
            argv: vec![
                "a b".to_owned(),
                String::new(),
                "tab\there".to_owned(),
                "line\nbreak\r\n".to_owned(),
                "\\".to_owned(),
                "\\t".to_owned(),
            ],
        };
        let line = odd.to_line();
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.ends_with('\n'));
        assert_eq!(Miss::parse(line.trim_end_matches('\n')), Some(odd));

        let bare = miss("x", "unknown script");
        let line = bare.to_line();
        assert_eq!(line.split('\t').count(), 4);
        assert_eq!(Miss::parse(line.trim_end_matches('\n')), Some(bare));

        let empty_arg = Miss {
            argv: vec![String::new()],
            ..miss("x", "r")
        };
        let line = empty_arg.to_line();
        assert_eq!(Miss::parse(line.trim_end_matches('\n')), Some(empty_arg));
    }

    #[test]
    fn malformed_lines_do_not_parse() {
        for line in ["", "a\tb\tc", "a\tb\tc\td\\", "a\tb\tc\\x\td"] {
            assert_eq!(Miss::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn cap_keeps_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canif").join(FILE_NAME);
        for index in 0..5 {
            append(&path, format!("{index}\n").as_bytes(), 3).unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "2\n3\n4\n");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE_NAME]);
    }

    #[test]
    fn summary_groups_by_name_then_reason() {
        let mut content = String::new();
        for (name, reason) in [
            ("cargo/test", "arg `count`: b"),
            ("cargo/tets", "unknown script"),
            ("cargo/test", "arg `crate`: a"),
            ("cargo/tets", "unknown script"),
            ("cargo/tets", "unknown script"),
            ("zzz", "unknown script"),
            ("aaa", "unknown script"),
        ] {
            content.push_str(&miss(name, reason).to_line());
        }
        content.push_str("malformed\n");
        assert_eq!(
            summary(&content),
            "3  cargo/tets
     3  unknown script
2  cargo/test
     1  arg `count`: b
     1  arg `crate`: a
1  aaa
     1  unknown script
1  zzz
     1  unknown script
"
        );
        assert_eq!(summary(""), "no misses recorded\n");
        assert_eq!(summary("junk\n"), "no misses recorded\n");
    }
}
