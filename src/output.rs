use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const LOGS_DIR: &str = ".logs";
/// After the script exits, output stops being relayed once no line arrives for this long.
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(100);
const KEPT_LOGS: usize = 50;
const LOG_EXTENSION: &str = "log";

/// Lines of the merged output to print: the first `head`, the last `tail`, all when both are unset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Trim {
    pub head: Option<NonZeroUsize>,
    pub tail: Option<NonZeroUsize>,
}

impl Trim {
    /// Per-call values replace the header's, key by key.
    pub fn overridden_by(self, call: Trim) -> Trim {
        Trim {
            head: call.head.or(self.head),
            tail: call.tail.or(self.tail),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Decides line by line what is printed live, and holds the tail until the end.
pub struct Trimmer<T> {
    head: Option<usize>,
    tail: Option<usize>,
    total: usize,
    dropped: usize,
    held: VecDeque<T>,
}

pub struct Trimmed<T> {
    pub total: usize,
    pub dropped: usize,
    /// Printed after the output ends, following the trimmed notice.
    pub tail: Vec<T>,
}

impl<T> Trimmer<T> {
    pub fn new(trim: Trim) -> Self {
        Self {
            head: trim.head.map(NonZeroUsize::get),
            tail: trim.tail.map(NonZeroUsize::get),
            total: 0,
            dropped: 0,
            held: VecDeque::new(),
        }
    }

    /// Returns the line back when it is printed right away.
    pub fn push(&mut self, line: T) -> Option<T> {
        self.total += 1;
        if self.head.is_none() && self.tail.is_none() {
            return Some(line);
        }
        if self.total <= self.head.unwrap_or(0) {
            return Some(line);
        }
        match self.tail {
            Some(tail) => {
                self.held.push_back(line);
                if self.held.len() > tail {
                    self.held.pop_front();
                    self.dropped += 1;
                }
            }
            None => self.dropped += 1,
        }
        None
    }

    pub fn finish(self) -> Trimmed<T> {
        Trimmed {
            total: self.total,
            dropped: self.dropped,
            tail: self.held.into(),
        }
    }
}

enum Message {
    Line(Stream, Vec<u8>),
    Exited(io::Result<ExitStatus>),
}

/// Relays the child's piped output to canif's own streams, trimmed, and logs all of it.
/// Returns once the child exited and its output drained.
pub fn relay(
    mut child: Child,
    trim: Trim,
    logs_dir: Option<&Path>,
    script_name: &str,
) -> io::Result<ExitStatus> {
    let (sender, receiver) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_reader(stdout, Stream::Stdout, sender.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_reader(stderr, Stream::Stderr, sender.clone());
    }
    thread::spawn(move || {
        let _ = sender.send(Message::Exited(child.wait()));
    });
    let started = Instant::now();
    let mut log = Log::open(logs_dir, script_name, started);
    let mut console = Console::default();
    if let Some(path) = log.path() {
        console.notice(&format!("canif: log {}", path.display()));
    }
    let mut trimmer = Trimmer::new(trim);
    let mut exit = None;
    loop {
        // Background processes can inherit the pipes and keep them open past the script's exit.
        let message = match exit {
            None => receiver.recv().ok(),
            Some(_) => receiver.recv_timeout(DRAIN_AFTER_EXIT).ok(),
        };
        match message {
            Some(Message::Line(stream, line)) => {
                log.write_line(&line);
                if let Some(line) = trimmer.push((stream, line)) {
                    console.write(line.0, &line.1);
                }
            }
            Some(Message::Exited(result)) => exit = Some(result),
            None => break,
        }
    }
    if let Some(result) = &exit {
        log.write_line(exit_line(result, started.elapsed()).as_bytes());
    }
    let log_path = log.finish();
    let trimmed = trimmer.finish();
    if trimmed.dropped > 0 {
        let location = match &log_path {
            Some(path) => format!("full log: {}", path.display()),
            None => "no log available".to_owned(),
        };
        console.notice(&format!(
            "canif: output trimmed ({} of {} lines), {location}",
            trimmed.dropped, trimmed.total
        ));
    }
    for (stream, line) in trimmed.tail {
        console.write(stream, &line);
    }
    if let Some(dir) = logs_dir {
        log.prune(dir);
    }
    // The waiter holds a sender until it sends, so the loop cannot end before `exit` is set.
    exit.unwrap_or_else(|| Err(io::Error::other("script exit status lost")))
}

fn exit_line(result: &io::Result<ExitStatus>, elapsed: Duration) -> String {
    let outcome = match result {
        Ok(status) => match status.code() {
            Some(code) => format!("exited {code}"),
            None => "killed by a signal".to_owned(),
        },
        Err(error) => format!("lost its exit status ({error})"),
    };
    format!("canif: {outcome} after {:.1}s\n", elapsed.as_secs_f64())
}

/// Not joined: after the script exits, a reader may block on a pipe a background process holds.
fn spawn_reader(reader: impl Read + Send + 'static, stream: Stream, sender: Sender<Message>) {
    thread::spawn(move || {
        read_lines(reader, |line| {
            let _ = sender.send(Message::Line(stream, line));
        })
    });
}

/// Splits on `\n`, keeping it; a last line without one is kept as is.
fn read_lines(reader: impl Read, mut on_line: impl FnMut(Vec<u8>)) {
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => on_line(line),
        }
    }
}

#[derive(Default)]
struct Console {
    /// The last byte written to stderr was not a newline, so a notice needs one first.
    stderr_mid_line: bool,
}

impl Console {
    fn write(&mut self, stream: Stream, bytes: &[u8]) {
        // Write errors (a closed pipe) are ignored so the child's output keeps draining.
        match stream {
            Stream::Stdout => {
                let mut stdout = io::stdout().lock();
                let _ = stdout.write_all(bytes).and_then(|()| stdout.flush());
            }
            Stream::Stderr => {
                let _ = io::stderr().lock().write_all(bytes);
                if let Some(last) = bytes.last() {
                    self.stderr_mid_line = *last != b'\n';
                }
            }
        }
    }

    fn notice(&mut self, text: &str) {
        let _ = io::stdout().flush();
        let separator = if self.stderr_mid_line { "\n" } else { "" };
        eprintln!("{separator}{text}");
        self.stderr_mid_line = false;
    }
}

/// Unbuffered, so the log of a running script can be read up to its latest line.
struct Log {
    file: Option<(PathBuf, File)>,
    started: Instant,
    /// The last line written had no newline, so the next one needs one first.
    mid_line: bool,
    warned: bool,
}

impl Log {
    fn open(dir: Option<&Path>, script_name: &str, started: Instant) -> Self {
        let mut log = Self {
            file: None,
            started,
            mid_line: false,
            warned: false,
        };
        if let Some(dir) = dir {
            let path = dir.join(log_file_name(script_name));
            match create_log(dir, &path) {
                Ok(file) => log.file = Some((path, file)),
                Err(error) => log.warn(&format!("cannot write log {}: {error}", path.display())),
            }
        }
        log
    }

    fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(|(path, _)| path.as_path())
    }

    /// Prefixes the line with the seconds since the start, in one write.
    fn write_line(&mut self, line: &[u8]) {
        let mut bytes = Vec::with_capacity(line.len() + 16);
        if self.mid_line {
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(elapsed_prefix(self.started.elapsed()).as_bytes());
        bytes.extend_from_slice(line);
        self.mid_line = line.last() != Some(&b'\n');
        if let Some((path, file)) = &mut self.file
            && let Err(error) = file.write_all(&bytes)
        {
            let message = format!("cannot write log {}: {error}", path.display());
            self.file = None;
            self.warn(&message);
        }
    }

    /// Flushes the log and returns its path, if it is complete.
    fn finish(&mut self) -> Option<PathBuf> {
        let (path, mut file) = self.file.take()?;
        match file.flush() {
            Ok(()) => Some(path),
            Err(error) => {
                self.warn(&format!("cannot write log {}: {error}", path.display()));
                None
            }
        }
    }

    fn prune(&mut self, dir: &Path) {
        if let Err(error) = prune_logs(dir, KEPT_LOGS) {
            self.warn(&format!("cannot prune logs in {}: {error}", dir.display()));
        }
    }

    fn warn(&mut self, message: &str) {
        if !self.warned {
            eprintln!("canif: {message}");
            self.warned = true;
        }
    }
}

fn elapsed_prefix(elapsed: Duration) -> String {
    format!("[{:7.1}s] ", elapsed.as_secs_f64())
}

fn create_log(dir: &Path, path: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    File::create_new(path)
}

/// Zero-padded millis sort by time; the pid keeps concurrent runs apart.
fn log_file_name(script_name: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    format!(
        "{millis:020}-{}-{}.{LOG_EXTENSION}",
        std::process::id(),
        script_name.replace('/', "_")
    )
}

/// Keeps the `kept` newest `.log` files by name, deletes the rest.
fn prune_logs(dir: &Path, kept: usize) -> io::Result<()> {
    let mut logs = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == LOG_EXTENSION)
        {
            logs.push(path);
        }
    }
    logs.sort();
    let excess = logs.len().saturating_sub(kept);
    for path in &logs[..excess] {
        match fs::remove_file(path) {
            // A concurrent run pruned it first.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trim(head: usize, tail: usize) -> Trim {
        Trim {
            head: NonZeroUsize::new(head),
            tail: NonZeroUsize::new(tail),
        }
    }

    /// Returns the lines printed live, then the result.
    fn run(trim: Trim, count: usize) -> (Vec<usize>, Trimmed<usize>) {
        let mut trimmer = Trimmer::new(trim);
        let live = (1..=count).filter_map(|line| trimmer.push(line)).collect();
        (live, trimmer.finish())
    }

    #[test]
    fn untrimmed_prints_everything_live() {
        let (live, trimmed) = run(Trim::default(), 5);
        assert_eq!(live, [1, 2, 3, 4, 5]);
        assert_eq!((trimmed.total, trimmed.dropped), (5, 0));
        assert!(trimmed.tail.is_empty());
    }

    #[test]
    fn head_only() {
        let (live, trimmed) = run(trim(2, 0), 5);
        assert_eq!(live, [1, 2]);
        assert_eq!((trimmed.total, trimmed.dropped), (5, 3));
        assert!(trimmed.tail.is_empty());

        let (live, trimmed) = run(trim(5, 0), 3);
        assert_eq!(live, [1, 2, 3]);
        assert_eq!(trimmed.dropped, 0);
    }

    #[test]
    fn tail_only() {
        let (live, trimmed) = run(trim(0, 2), 5);
        assert!(live.is_empty());
        assert_eq!((trimmed.total, trimmed.dropped), (5, 3));
        assert_eq!(trimmed.tail, [4, 5]);
    }

    #[test]
    fn head_and_tail() {
        let (live, trimmed) = run(trim(2, 2), 7);
        assert_eq!(live, [1, 2]);
        assert_eq!((trimmed.total, trimmed.dropped), (7, 3));
        assert_eq!(trimmed.tail, [6, 7]);
    }

    #[test]
    fn head_and_tail_overlap_prints_everything_once() {
        for count in [0, 1, 3, 4] {
            let (live, trimmed) = run(trim(2, 2), count);
            let mut printed = live;
            printed.extend(trimmed.tail);
            assert_eq!(printed, (1..=count).collect::<Vec<_>>(), "{count}");
            assert_eq!(trimmed.dropped, 0);
        }
    }

    #[test]
    fn call_overrides_header_per_key() {
        let header = trim(3, 4);
        assert_eq!(header.overridden_by(trim(0, 9)), trim(3, 9));
        assert_eq!(header.overridden_by(trim(1, 0)), trim(1, 4));
        assert_eq!(Trim::default().overridden_by(trim(2, 0)), trim(2, 0));
    }

    fn lines_of(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut lines = Vec::new();
        read_lines(bytes, |line| lines.push(line));
        lines
    }

    #[test]
    fn lines_keep_newlines_and_a_final_partial_line() {
        assert_eq!(lines_of(b"a\nb\r\nc"), [&b"a\n"[..], b"b\r\n", b"c"]);
        assert_eq!(lines_of(b"a\n"), [b"a\n"]);
        assert!(lines_of(b"").is_empty());
        assert_eq!(lines_of(b"\xff\xfe\n\x80"), [&b"\xff\xfe\n"[..], b"\x80"]);
    }

    #[test]
    fn elapsed_prefix_pads_to_a_fixed_width() {
        assert_eq!(elapsed_prefix(Duration::from_millis(0)), "[    0.0s] ");
        assert_eq!(
            elapsed_prefix(Duration::from_millis(2_412_340)),
            "[ 2412.3s] "
        );
    }

    #[test]
    fn log_names_sort_by_time_and_flatten_script_names() {
        let name = log_file_name("cargo/test");
        let suffix = format!("-{}-cargo_test.log", std::process::id());
        assert!(name.ends_with(&suffix), "{name}");
        assert_eq!(name.len(), 20 + suffix.len());
    }

    #[test]
    fn prune_keeps_newest_logs_only() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..5 {
            fs::write(dir.path().join(format!("{index:020}-1-x.log")), "").unwrap();
        }
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        prune_logs(dir.path(), 3).unwrap();
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                format!("{:020}-1-x.log", 2),
                format!("{:020}-1-x.log", 3),
                format!("{:020}-1-x.log", 4),
                "notes.txt".to_owned(),
            ]
        );
    }
}
