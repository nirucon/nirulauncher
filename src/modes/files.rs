//! File search (`--files`, and `/ query` in the unified view).
//!
//! The roots (the home directory by default) are searched with `plocate` or
//! `locate` when their database covers the roots, else with `fd`/`fdfind`,
//! else with a bounded walker that lists the roots once per open. Recently
//! used files (`recently-used.xbel`) come first. Every search runs in a
//! worker that a newer query cancels, with a time limit and a result cap, so
//! typing never waits for the disk. No daemon and no index of our own.

use crate::*;
use config::FilesConfig;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Row keys are `file:` and the absolute path.
pub(crate) const KEY_PREFIX: &str = "file:";
/// The longest a search may take before it shows what it has.
const DEADLINE: Duration = Duration::from_millis(1500);
/// Typing faster than this starts one search, not one per key.
const DEBOUNCE: Duration = Duration::from_millis(60);
/// Paths read from `fd` before ranking.
const FD_CANDIDATES: usize = 5000;
/// Paths read from `locate` before ranking; its output includes hidden and
/// excluded paths that are dropped afterwards.
const LOCATE_CANDIDATES: usize = 20000;
/// The built-in walker lists at most this many entries, within this time.
const WALK_ENTRIES: usize = 250_000;
const WALK_TIME: Duration = Duration::from_secs(8);
/// Recently used files read from the bookmark file.
const RECENT_MAX: usize = 100;
/// The bookmark file is read up to this size.
const RECENT_BYTES: u64 = 8 << 20;
/// Longest query that is searched, in characters.
const MAX_QUERY: usize = 120;

/// How the roots are searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    /// `plocate` or `locate`; `all` adds `-A`, which mlocate needs to
    /// require every word (plocate always does).
    Locate {
        program: String,
        all: bool,
    },
    /// `fd` or `fdfind`; `and` when it has `--and` (fd 8.6 or newer) to
    /// require every word.
    Fd {
        program: String,
        and: bool,
    },
    Builtin,
}

impl Backend {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Locate { program, .. } | Self::Fd { program, .. } => program,
            Self::Builtin => "built-in search",
        }
    }
}

/// One search's rows and what the footer says about it.
pub(crate) struct Reply {
    pub(crate) query: String,
    pub(crate) items: Vec<Item>,
    /// The first `recent` rows are recently used files.
    pub(crate) recent: usize,
    pub(crate) note: String,
    /// The built-in walker had not finished listing; search again when it has.
    pub(crate) partial: bool,
}

/// The file search state of an open launcher.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) rx: Option<Receiver<Reply>>,
    cancel: Option<Arc<AtomicBool>>,
    /// The query the rows belong to, or are being searched for.
    pub(crate) sent: Option<String>,
    pub(crate) rows: Vec<Item>,
    pub(crate) recent: usize,
    pub(crate) note: String,
    pub(crate) partial: bool,
}

impl State {
    /// Start a search for `query` unless it is the one already shown or
    /// running. The previous search is cancelled.
    pub(crate) fn request(&mut self, cfg: &FilesConfig, query: &str) {
        if self.sent.as_deref() == Some(query) {
            return;
        }
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let job_cancel = Arc::clone(&cancel);
        let cfg = cfg.clone();
        let text = query.to_string();
        thread::spawn(move || {
            let reply = search(&cfg, &text, &job_cancel);
            if !job_cancel.load(Ordering::Relaxed) {
                let _ = tx.send(reply);
            }
        });
        self.cancel = Some(cancel);
        self.rx = Some(rx);
        self.sent = Some(query.to_string());
        self.partial = false;
    }

    /// Take a finished search. True when the rows changed.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.as_ref() else {
            return false;
        };
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                self.note = "File search stopped".into();
                return true;
            }
        };
        self.rx = None;
        self.cancel = None;
        if self.sent.as_deref() != Some(reply.query.as_str()) {
            return false;
        }
        self.rows = reply.items;
        self.recent = reply.recent;
        self.note = reply.note;
        self.partial = reply.partial;
        true
    }

    /// Search the same query again (the walker has finished listing).
    pub(crate) fn again(&mut self, cfg: &FilesConfig) {
        if let Some(query) = self.sent.take() {
            self.request(cfg, &query);
        }
    }

    /// Leave file search: cancel a running search and forget the rows.
    pub(crate) fn reset(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        *self = Self::default();
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The query of a `/ query` prefix in the unified views.
pub(crate) fn prefixed(query: &str) -> Option<&str> {
    query.trim_start().strip_prefix('/').map(str::trim)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// `~` and `~/x` relative to the home directory; other relative roots are
/// taken below it too. Each root is resolved once (symlinks), so paths from
/// every backend share the same prefix.
pub(crate) fn roots(cfg: &FilesConfig) -> Vec<PathBuf> {
    let home = home();
    let mut out: Vec<PathBuf> = Vec::new();
    for root in &cfg.roots {
        let path = if root == "~" {
            home.clone()
        } else if let Some(rest) = root.strip_prefix("~/") {
            home.as_ref().map(|h| h.join(rest))
        } else if Path::new(root).is_absolute() {
            Some(PathBuf::from(root))
        } else {
            home.as_ref().map(|h| h.join(root))
        };
        let Some(path) = path else { continue };
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if path.is_dir() && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// `*` and `?` glob on one name or relative path.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Whether one more component (`name`, reached by `rel` from the root) may
/// be searched: not hidden (unless allowed) and not excluded.
fn component_allowed(name: &str, rel: &str, cfg: &FilesConfig) -> bool {
    if !cfg.show_hidden && name.starts_with('.') {
        return false;
    }
    !cfg.exclude.iter().any(|pattern| {
        if pattern.contains('/') {
            glob(pattern, rel)
        } else {
            glob(pattern, name)
        }
    })
}

/// Whether a path below a root, `a/b/c`, is shown: every component must be.
pub(crate) fn path_allowed(rel: &str, cfg: &FilesConfig) -> bool {
    let mut end = 0;
    for name in rel.split('/') {
        end += name.len();
        if !name.is_empty() && !component_allowed(name, &rel[..end], cfg) {
            return false;
        }
        end += 1;
    }
    true
}

/// The query as lowercase words, and the same words with diacritics folded.
pub(crate) struct Terms {
    exact: Vec<String>,
    folded: Vec<String>,
}

impl Terms {
    pub(crate) fn new(query: &str) -> Self {
        let query: String = query.chars().take(MAX_QUERY).collect();
        let exact: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let folded = exact.iter().map(|t| compose::fold(t)).collect();
        Self { exact, folded }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.exact.is_empty()
    }

    /// The words a backend is asked for, longest (narrowest) first.
    fn by_length(&self) -> Vec<&str> {
        let mut words: Vec<&str> = self.folded.iter().map(String::as_str).collect();
        words.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        words.dedup();
        words
    }

    /// How well a path below a root (`rel`, lowercase) matches every word;
    /// lower is better. A word in the file name beats one in a folder name,
    /// and a match on the exact text beats one that needed folding.
    pub(crate) fn score(&self, rel: &str) -> Option<u32> {
        let rel_folded = compose::fold(rel);
        let mut total = 0u32;
        for (exact, folded) in self.exact.iter().zip(&self.folded) {
            let a = word_score(exact, rel).map(|s| s * 2);
            let b = word_score(folded, &rel_folded).map(|s| s * 2 + 1);
            total += u32::from(match (a, b) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b)?,
            });
        }
        Some(total)
    }
}

/// One word against one lowercase relative path.
fn word_score(term: &str, rel: &str) -> Option<u8> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if name == term || stem == term {
        Some(0)
    } else if name.starts_with(term) {
        Some(1)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(term))
    {
        Some(2)
    } else if name.contains(term) {
        Some(3)
    } else if rel.contains(term) {
        Some(5)
    } else {
        None
    }
}

/// Characters a lowercase base letter stands for: itself and its accented
/// forms in both cases (case itself is left to the backend's flag).
fn variants(base: char) -> Vec<char> {
    static TABLE: OnceLock<Vec<(char, Vec<char>)>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table: Vec<(char, Vec<char>)> = Vec::new();
        for code in 0xc0u32..=0x17f {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let lower = ch.to_lowercase().next().unwrap_or(ch);
            let folded = compose::fold_char(lower);
            if folded == lower || !folded.is_ascii_lowercase() {
                continue;
            }
            match table.iter_mut().find(|(b, _)| *b == folded) {
                Some((_, list)) => {
                    if !list.contains(&ch) {
                        list.push(ch);
                    }
                }
                None => table.push((folded, vec![ch])),
            }
        }
        table
    });
    let mut out = vec![base];
    if let Some((_, list)) = table.iter().find(|(b, _)| *b == base) {
        out.extend(list.iter().copied());
    }
    out
}

/// Escape for both POSIX extended and Rust regular expressions.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A pattern for a folded word that also matches its accented letters
/// (`a` → `(a|A|à|…|å)`); both regex dialects accept the alternation.
pub(crate) fn word_regex(folded: &str) -> String {
    let mut out = String::new();
    for ch in folded.chars() {
        let alts = variants(ch);
        if alts.len() > 1 {
            out.push('(');
            let parts: Vec<String> = alts.iter().map(|c| escape(&c.to_string())).collect();
            out.push_str(&parts.join("|"));
            out.push(')');
        } else {
            out.push_str(&escape(&ch.to_string()));
        }
    }
    out
}

/// `^(root1|root2)/.*word`: the word somewhere below one of the roots.
pub(crate) fn path_regex(roots: &[PathBuf], folded: &str) -> String {
    let roots: Vec<String> = roots
        .iter()
        .map(|r| escape(r.to_string_lossy().trim_end_matches('/')))
        .collect();
    format!("^({})/.*{}", roots.join("|"), word_regex(folded))
}

/// Run a program and hand each NUL-separated path to `accept` until it says
/// enough, the deadline passes or the search is cancelled. argv only.
/// Returns false when it stopped early.
fn read_paths(
    argv: &[String],
    cancel: &AtomicBool,
    deadline: Instant,
    mut accept: impl FnMut(String) -> bool,
) -> std::result::Result<bool, String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", argv[0]))?;
    let stdout = child.stdout.take().ok_or("no output")?;
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(1024);
    let reader = thread::spawn(move || {
        let mut input = std::io::BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match input.read_until(0, &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.last() == Some(&0) {
                        line.pop();
                    }
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut complete = true;
    loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            complete = false;
            break;
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(line) => {
                // A name that is not UTF-8 cannot be shown or opened by argv
                // text; it is skipped.
                if let Ok(path) = String::from_utf8(line) {
                    if !accept(path) {
                        complete = false;
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if !complete {
        let _ = child.kill();
    }
    drop(rx);
    let _ = child.wait();
    let _ = reader.join();
    Ok(complete)
}

/// Whether `program`'s database covers every root: it knows a path below
/// each. Asked once per root and process, with a short time limit.
fn locate_covers(program: &str, roots: &[PathBuf]) -> bool {
    let never = AtomicBool::new(false);
    roots.iter().all(|root| {
        let prefix = format!("{}/", root.to_string_lossy().trim_end_matches('/'));
        let mut found = false;
        let argv = vec![
            program.to_string(),
            "-0".into(),
            "-l".into(),
            "1".into(),
            "--".into(),
            prefix.clone(),
        ];
        let deadline = Instant::now() + Duration::from_millis(800);
        let _ = read_paths(&argv, &never, deadline, |path| {
            found |= path.starts_with(&prefix);
            !found
        });
        found
    })
}

/// The backend `cfg.backend` asks for, or the first available one for
/// `auto` (and when the asked-for one is missing, with a note why).
pub(crate) fn resolve(cfg: &FilesConfig, roots: &[PathBuf]) -> (Backend, Option<String>) {
    let locate = |program: &str| {
        if !in_path(program) {
            return None;
        }
        // GNU findutils' locate has another regex dialect; it is not used.
        let version = version_line(program);
        let all = version.starts_with("mlocate");
        let known = version.starts_with("plocate") || all;
        (known && locate_covers(program, roots)).then(|| Backend::Locate {
            program: program.into(),
            all,
        })
    };
    let fd = |program: &str| {
        in_path(program).then(|| Backend::Fd {
            program: program.into(),
            and: fd_has_and(&version_line(program)),
        })
    };
    let auto = || {
        locate("plocate")
            .or_else(|| locate("locate"))
            .or_else(|| fd("fdfind"))
            .or_else(|| fd("fd"))
            .unwrap_or(Backend::Builtin)
    };
    let wanted = match cfg.backend.as_str() {
        "auto" | "" => return (auto(), None),
        "builtin" | "built-in" | "walk" => return (Backend::Builtin, None),
        "plocate" | "locate" => locate(&cfg.backend),
        "fd" | "fdfind" => fd(&cfg.backend)
            .or_else(|| fd("fdfind"))
            .or_else(|| fd("fd")),
        other => {
            return (
                auto(),
                Some(format!("unknown files backend {other:?}, using auto")),
            )
        }
    };
    match wanted {
        Some(backend) => (backend, None),
        None => (
            auto(),
            Some(format!("{} is not available, using auto", cfg.backend)),
        ),
    }
}

/// The first line of `program --version`, within a short time limit.
fn version_line(program: &str) -> String {
    let never = AtomicBool::new(false);
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_millis(800);
    let argv = [program.to_string(), "--version".into()];
    let _ = read_paths(&argv, &never, deadline, |chunk| {
        text.push_str(&chunk);
        text.len() < 4096
    });
    text.lines().next().unwrap_or("").trim().to_string()
}

/// `fd 10.2.0` and `fdfind 8.6.0` have `--and`; 8.5 and older do not.
fn fd_has_and(version: &str) -> bool {
    let number = version.split_whitespace().nth(1).unwrap_or("");
    let mut parts = number.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    (major, minor) >= (8, 6)
}

/// The backend of this process, chosen once.
fn backend(cfg: &FilesConfig, roots: &[PathBuf]) -> (Backend, Option<String>) {
    static RESOLVED: OnceLock<(Backend, Option<String>)> = OnceLock::new();
    RESOLVED.get_or_init(|| resolve(cfg, roots)).clone()
}

/// When the file view opens: choose the backend and, for the built-in one,
/// start listing, in a worker, so the first search does not wait for it.
pub(crate) fn prefetch(cfg: &FilesConfig) {
    if !cfg.enabled {
        return;
    }
    let cfg = cfg.clone();
    thread::spawn(move || {
        let roots = roots(&cfg);
        if backend(&cfg, &roots).0 == Backend::Builtin {
            walk(&cfg, &roots);
        }
    });
}

/// Paths below the roots that may match: `fd` or `locate` are asked for the
/// longest word; the caller checks the rest.
fn backend_paths(
    backend: &Backend,
    cfg: &FilesConfig,
    roots: &[PathBuf],
    terms: &Terms,
    cancel: &AtomicBool,
    deadline: Instant,
) -> std::result::Result<(Vec<String>, bool), String> {
    let words = terms.by_length();
    let regexes: Vec<String> = words.iter().map(|w| path_regex(roots, w)).collect();
    let (argv, cap) = match backend {
        Backend::Locate { program, all } => {
            let mut argv = vec![
                program.clone(),
                "-0".into(),
                "-i".into(),
                "-l".into(),
                LOCATE_CANDIDATES.to_string(),
            ];
            if *all {
                argv.push("-A".into());
            }
            // Every word is a pattern and every pattern must match.
            argv.push("--regex".into());
            argv.push("--".into());
            argv.extend(regexes);
            (argv, LOCATE_CANDIDATES)
        }
        Backend::Fd { program, and } => {
            let mut argv = vec![
                program.clone(),
                "--color".into(),
                "never".into(),
                "--absolute-path".into(),
                "--no-ignore".into(),
                "--one-file-system".into(),
                "--ignore-case".into(),
                "--print0".into(),
                "--max-results".into(),
                FD_CANDIDATES.to_string(),
            ];
            if cfg.show_hidden {
                argv.push("--hidden".into());
            }
            for pattern in &cfg.exclude {
                argv.push("--exclude".into());
                argv.push(pattern.clone());
            }
            argv.push("--full-path".into());
            // The other words with `--and`; an older fd gets the longest word
            // and the rest are checked here.
            if *and {
                for regex in regexes.iter().skip(1) {
                    argv.push("--and".into());
                    argv.push(regex.clone());
                }
            }
            argv.push("--".into());
            argv.push(regexes.first().cloned().unwrap_or_default());
            argv.extend(roots.iter().map(|r| r.to_string_lossy().into_owned()));
            (argv, FD_CANDIDATES)
        }
        Backend::Builtin => return Ok((Vec::new(), true)),
    };
    let mut paths = Vec::new();
    let complete = read_paths(&argv, cancel, deadline, |path| {
        paths.push(path);
        paths.len() < cap
    })?;
    let complete = complete && paths.len() < cap;
    Ok((paths, complete))
}

/// What the built-in walker has listed so far, shared by the searches of
/// one open launcher.
struct Walk {
    /// `(absolute path, lowercase path below its root, is a folder)`.
    entries: Mutex<Vec<(String, String, bool)>>,
    done: AtomicBool,
    truncated: AtomicBool,
}

static WALK: OnceLock<Arc<Walk>> = OnceLock::new();

/// Start listing the roots once; later calls return the same listing.
fn walk(cfg: &FilesConfig, roots: &[PathBuf]) -> Arc<Walk> {
    let mut started = false;
    let walk = WALK.get_or_init(|| {
        started = true;
        Arc::new(Walk {
            entries: Mutex::new(Vec::new()),
            done: AtomicBool::new(false),
            truncated: AtomicBool::new(false),
        })
    });
    if started {
        let walk = Arc::clone(walk);
        let cfg = cfg.clone();
        let roots = roots.to_vec();
        thread::spawn(move || walk_roots(&cfg, &roots, &walk));
    }
    Arc::clone(walk)
}

/// True while the built-in walker is still listing.
pub(crate) fn walk_running() -> bool {
    WALK.get().is_some_and(|w| !w.done.load(Ordering::Relaxed))
}

/// Breadth first, so shallow paths are listed first; one file system per
/// root, symlinks are not followed, hidden and excluded names are skipped.
fn walk_roots(cfg: &FilesConfig, roots: &[PathBuf], walk: &Walk) {
    use std::os::unix::fs::MetadataExt;
    let started = Instant::now();
    let mut count = 0usize;
    'roots: for root in roots {
        let Ok(meta) = std::fs::metadata(root) else {
            continue;
        };
        let device = meta.dev();
        let mut queue = std::collections::VecDeque::from([(root.clone(), String::new())]);
        while let Some((dir, rel)) = queue.pop_front() {
            if count >= WALK_ENTRIES || started.elapsed() > WALK_TIME {
                walk.truncated.store(true, Ordering::Relaxed);
                break 'roots;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut batch = Vec::new();
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let child_rel = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                if !component_allowed(&name, &child_rel, cfg) {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                let is_dir = kind.is_dir();
                if is_dir && entry.metadata().map(|m| m.dev() == device).unwrap_or(false) {
                    queue.push_back((path.clone(), child_rel.clone()));
                }
                if let Some(text) = path.to_str() {
                    batch.push((text.to_string(), child_rel.to_lowercase(), is_dir));
                }
            }
            count += batch.len();
            if let Ok(mut list) = walk.entries.lock() {
                list.extend(batch);
            }
        }
    }
    walk.done.store(true, Ordering::Relaxed);
}

/// `recently-used.xbel` in the data directory.
fn recent_path() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".local/share")))
        .map(|d| d.join("recently-used.xbel"))
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

/// Local files in a bookmark file, most recently used first.
pub(crate) fn parse_recent(text: &str) -> Vec<String> {
    let mut found: Vec<(String, String)> = Vec::new();
    for chunk in text.split("<bookmark ").skip(1) {
        let tag = format!(" {}", chunk.split('>').next().unwrap_or(""));
        let Some(href) = attribute(&tag, "href") else {
            continue;
        };
        let href = xml_unescape(href);
        let Some(path) = href.strip_prefix("file://").and_then(percent_decode) else {
            continue;
        };
        if !path.starts_with('/') {
            continue;
        }
        let stamp = [
            attribute(&tag, "visited"),
            attribute(&tag, "modified"),
            attribute(&tag, "added"),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or("")
        .to_string();
        found.push((stamp, path));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    found
        .into_iter()
        .filter(|(_, path)| seen.insert(path.clone()))
        .map(|(_, path)| path)
        .collect()
}

/// Recently used files that still exist, read once per process.
fn recent_files() -> &'static [String] {
    static RECENT: OnceLock<Vec<String>> = OnceLock::new();
    RECENT.get_or_init(|| {
        let Some(path) = recent_path() else {
            return Vec::new();
        };
        let mut text = String::new();
        let read = std::fs::File::open(path)
            .map(|f| std::io::Read::take(f, RECENT_BYTES).read_to_string(&mut text));
        if !matches!(read, Ok(Ok(_))) {
            return Vec::new();
        }
        parse_recent(&text)
            .into_iter()
            .filter(|p| Path::new(p).exists())
            .take(RECENT_MAX)
            .collect()
    })
}

/// The kind shown before a file's name.
pub(crate) fn kind(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "Folder";
    }
    let ext = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "PDF",
        "doc" | "docx" | "odt" | "rtf" => "Document",
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => "Spreadsheet",
        "ppt" | "pptx" | "odp" | "key" => "Slides",
        "txt" | "md" | "org" | "log" | "tex" => "Text",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic"
        | "avif" | "xcf" | "kra" | "psd" | "raw" | "cr2" | "nef" | "dng" => "Image",
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" | "aac" | "mid" | "midi" => "Audio",
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "mpg" | "mpeg" => "Video",
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm" => {
            "Archive"
        }
        "rs" | "py" | "sh" | "c" | "h" | "cpp" | "hpp" | "js" | "ts" | "go" | "java" | "lua"
        | "rb" | "pl" | "toml" | "json" | "yaml" | "yml" | "xml" | "html" | "css" | "fish" => {
            "Code"
        }
        "iso" | "img" | "qcow2" => "Disk image",
        "ttf" | "otf" | "woff" | "woff2" => "Font",
        "epub" | "mobi" | "azw3" | "cbz" | "cbr" => "Book",
        "desktop" | "appimage" | "exe" | "msi" => "Program",
        _ => "File",
    }
}

/// The folder a path is in, as `~/a/b`, shortened from the left to `max`
/// characters (`~/…/b/c`).
pub(crate) fn short_parent(path: &str, home: Option<&Path>, max: usize) -> String {
    let parent = Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = match home.map(|h| h.to_string_lossy().into_owned()) {
        Some(h) if parent == h => "~".to_string(),
        Some(h) if parent.starts_with(&format!("{h}/")) => format!("~{}", &parent[h.len()..]),
        _ => parent,
    };
    if shown.chars().count() <= max {
        return shown;
    }
    let (lead, rest) = if let Some(rest) = shown.strip_prefix("~/") {
        ("~/…/", rest)
    } else {
        ("…/", shown.trim_start_matches('/'))
    };
    let parts: Vec<&str> = rest.split('/').collect();
    let mut tail: Vec<&str> = Vec::new();
    let mut len = lead.chars().count();
    for part in parts.iter().rev() {
        let add = part.chars().count() + usize::from(!tail.is_empty());
        if len + add > max && !tail.is_empty() {
            break;
        }
        len += add;
        tail.insert(0, part);
    }
    let text = format!("{lead}{}", tail.join("/"));
    if text.chars().count() > max {
        // One very long folder name: keep its end.
        let skip = text.chars().count() - max + 1;
        format!("…{}", text.chars().skip(skip).collect::<String>())
    } else {
        text
    }
}

/// The parent folder of a path, for Ctrl+Enter.
pub(crate) fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".into())
}

pub(crate) fn item(path: &str, is_dir: bool, home: Option<&Path>) -> Item {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    Item {
        key: format!("{KEY_PREFIX}{path}"),
        group: kind(&name, is_dir).into(),
        detail: short_parent(path, home, 36),
        name,
        keywords: String::new(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["xdg-open".into(), path.to_string()],
            terminal: false,
            path: None,
        },
    }
}

/// The text a path is matched on: below its root, else the whole path.
fn relative<'a>(path: &'a str, roots: &[PathBuf]) -> Option<&'a str> {
    roots
        .iter()
        .filter_map(|root| {
            let root = root.to_str()?.trim_end_matches('/');
            path.strip_prefix(root)?.strip_prefix('/')
        })
        .min_by_key(|rel| rel.len())
}

fn sleep_unless_cancelled(time: Duration, cancel: &AtomicBool) -> bool {
    let end = Instant::now() + time;
    while Instant::now() < end {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    !cancel.load(Ordering::Relaxed)
}

/// Search the roots for `query`: recent files first, then the best matches.
pub(crate) fn search(cfg: &FilesConfig, query: &str, cancel: &AtomicBool) -> Reply {
    let started = Instant::now();
    let reply = |items: Vec<Item>, recent: usize, note: String, partial: bool| Reply {
        query: query.to_string(),
        items,
        recent,
        note,
        partial,
    };
    if !cfg.enabled {
        return reply(
            vec![info_row(
                "File search is turned off",
                "[files] enabled = false",
            )],
            0,
            String::new(),
            false,
        );
    }
    let terms = Terms::new(query);
    let home = home();
    let max = cfg.max_results.max(1);
    if terms.is_empty() {
        let items: Vec<Item> = recent_files()
            .iter()
            .take(max)
            .map(|p| item(p, Path::new(p).is_dir(), home.as_deref()))
            .collect();
        if items.is_empty() {
            return reply(
                vec![info_row(
                    "No recent files",
                    "type a file or folder name to search",
                )],
                0,
                String::new(),
                false,
            );
        }
        let count = items.len();
        return reply(items, count, "Recent files".into(), false);
    }
    if !sleep_unless_cancelled(DEBOUNCE, cancel) {
        return reply(Vec::new(), 0, String::new(), false);
    }
    let roots = roots(cfg);
    let (backend, warning) = backend(cfg, &roots);
    let deadline = started + DEADLINE;
    // (recent?, score, depth, length, path, is_dir if known)
    let mut hits: Vec<(bool, u32, usize, usize, String, Option<bool>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in recent_files() {
        let rel = relative(path, &roots).unwrap_or(path.trim_start_matches('/'));
        if let Some(score) = terms.score(&rel.to_lowercase()) {
            seen.insert(path.clone());
            hits.push((
                false,
                score,
                rel.matches('/').count(),
                rel.len(),
                path.clone(),
                None,
            ));
        }
    }
    let mut complete = true;
    let mut partial = false;
    let mut error = None;
    if backend == Backend::Builtin {
        let walk = walk(cfg, &roots);
        while !walk.done.load(Ordering::Relaxed) && Instant::now() < deadline {
            if !sleep_unless_cancelled(Duration::from_millis(20), cancel) {
                return reply(Vec::new(), 0, String::new(), false);
            }
        }
        partial = !walk.done.load(Ordering::Relaxed);
        complete = !walk.truncated.load(Ordering::Relaxed);
        // Scored under the lock: the walker has finished or waits briefly.
        let guard = walk.entries.lock();
        if let Ok(list) = guard.as_ref() {
            for (path, rel, is_dir) in list.iter() {
                if cancel.load(Ordering::Relaxed) {
                    return reply(Vec::new(), 0, String::new(), false);
                }
                if let Some(score) = terms.score(rel) {
                    if seen.insert(path.clone()) {
                        hits.push((
                            true,
                            score,
                            rel.matches('/').count(),
                            rel.len(),
                            path.clone(),
                            Some(*is_dir),
                        ));
                    }
                }
            }
        }
        drop(guard);
    } else {
        match backend_paths(&backend, cfg, &roots, &terms, cancel, deadline) {
            Ok((paths, done)) => {
                complete = done;
                for path in paths {
                    let Some(rel) = relative(&path, &roots) else {
                        continue;
                    };
                    if !path_allowed(rel, cfg) {
                        continue;
                    }
                    let rel_lower = rel.to_lowercase();
                    if let Some(score) = terms.score(&rel_lower) {
                        if seen.insert(path.clone()) {
                            hits.push((
                                true,
                                score,
                                rel.matches('/').count(),
                                rel.len(),
                                path,
                                None,
                            ));
                        }
                    }
                }
            }
            Err(e) => error = Some(e),
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return reply(Vec::new(), 0, String::new(), false);
    }
    hits.sort_by(|a, b| {
        (a.0, a.1, a.2, a.3)
            .cmp(&(b.0, b.1, b.2, b.3))
            .then_with(|| a.4.cmp(&b.4))
    });
    let mut items = Vec::new();
    let mut recent = 0;
    // A locate database can be older than the disk: rows that no longer
    // exist are left out.
    for (later, _, _, _, path, is_dir) in hits.into_iter().take(max * 3) {
        if items.len() >= max {
            break;
        }
        let is_dir = match is_dir {
            Some(d) => d,
            None => match std::fs::metadata(&path) {
                Ok(meta) => meta.is_dir(),
                Err(_) => continue,
            },
        };
        if !later {
            recent += 1;
        }
        items.push(item(&path, is_dir, home.as_deref()));
    }
    let ms = started.elapsed().as_millis();
    let mut note = match &error {
        Some(e) => e.clone(),
        None => format!(
            "{} {} · {} · {ms} ms",
            items.len(),
            if items.len() == 1 { "file" } else { "files" },
            backend.name()
        ),
    };
    if partial {
        note.push_str(" · still listing…");
    } else if !complete {
        note.push_str(" · limited");
    }
    if let Some(w) = warning {
        note = format!("{w} · {note}");
    }
    if items.is_empty() && error.is_none() {
        let shown: String = query.trim().chars().take(48).collect();
        items.push(info_row(&format!("No files match ‘{shown}’"), ""));
    }
    reply(items, recent, note, partial)
}

fn info_row(name: &str, detail: &str) -> Item {
    Item {
        key: "files:info".into(),
        group: String::new(),
        name: name.into(),
        detail: detail.into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }
}

/// `--list-files QUERY`: the rows a search finds, for scripts and tests.
pub(crate) fn print_search(cfg: &FilesConfig, query: &str) {
    let never = AtomicBool::new(false);
    let reply = search(cfg, query, &never);
    let reply = if reply.partial {
        // Wait for the walker so a script sees the whole answer.
        while walk_running() {
            thread::sleep(Duration::from_millis(20));
        }
        search(cfg, query, &never)
    } else {
        reply
    };
    for item in &reply.items {
        if let Some(path) = item.key.strip_prefix(KEY_PREFIX) {
            println!("{}\t{path}", item.group);
        }
    }
    eprintln!("{}", reply.note);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FilesConfig {
        FilesConfig::default()
    }

    #[test]
    fn hidden_and_excluded_paths_are_left_out() {
        let c = cfg();
        assert!(path_allowed("Fakturor/faktura-åäö.pdf", &c));
        assert!(!path_allowed(".config/x.txt", &c));
        assert!(!path_allowed("dev/app/node_modules/a/readme.md", &c));
        assert!(!path_allowed("dev/app/target/debug/app", &c));
        assert!(!path_allowed(".local/share/Trash/files/a.pdf", &c));
        assert!(!path_allowed("proj/.git/config", &c));
        let mut shown = cfg();
        shown.show_hidden = true;
        assert!(path_allowed(".config/x.txt", &shown));
        assert!(!path_allowed(".local/share/Trash/files/a.pdf", &shown));
        assert!(path_allowed(".local/share/other.txt", &shown));
        let mut own = cfg();
        own.exclude = vec!["*.o".into(), "Games".into()];
        assert!(!path_allowed("src/main.o", &own));
        assert!(!path_allowed("Games/x", &own));
        assert!(path_allowed("dev/target/x", &own));
    }

    #[test]
    fn fd_versions_with_and() {
        assert!(fd_has_and("fd 10.2.0"));
        assert!(fd_has_and("fdfind 8.6.0"));
        assert!(!fd_has_and("fd 8.5.3"));
        assert!(!fd_has_and("fd 7.4.0"));
        assert!(!fd_has_and(""));
    }

    #[test]
    fn globs_match_names() {
        assert!(glob("*.o", "main.o"));
        assert!(!glob("*.o", "main.on"));
        assert!(glob("a?c", "abc"));
        assert!(glob("node_modules", "node_modules"));
        assert!(!glob("node_modules", "node_modules2"));
        assert!(glob(".local/share/Trash", ".local/share/Trash"));
    }

    #[test]
    fn words_match_in_any_order_and_ignore_diacritics() {
        let t = Terms::new("faktura aao");
        assert!(t.score("fakturor/faktura-åäö.pdf").is_some());
        let t = Terms::new("Faktura ÅÄÖ");
        assert!(t.score("fakturor/faktura-åäö.pdf").is_some());
        let t = Terms::new("2024 fakturor");
        assert!(t.score("fakturor/2024/jan.pdf").is_some());
        assert!(Terms::new("zzz").score("fakturor/faktura.pdf").is_none());
        // The exact text ranks before the same match through folding.
        let exact = Terms::new("åäö").score("x/åäö.txt").unwrap();
        let folded = Terms::new("aao").score("x/åäö.txt").unwrap();
        assert!(exact < folded);
    }

    #[test]
    fn a_word_in_the_file_name_beats_one_in_a_folder() {
        let t = Terms::new("faktura");
        let name = t.score("docs/faktura.pdf").unwrap();
        let prefix = t.score("docs/faktura-mars.pdf").unwrap();
        let inside = t.score("docs/min-faktura-mars.pdf").unwrap();
        let folder = t.score("faktura/mars.pdf").unwrap();
        assert!(name < prefix && prefix < inside && inside < folder);
    }

    #[test]
    fn regexes_cover_accents_and_escape_symbols() {
        let r = word_regex("aao");
        assert!(r.contains('å') && r.contains('ä') && r.contains('Å') && r.contains('ö'));
        assert_eq!(word_regex("x.q"), "x\\.q");
        assert!(word_regex("(1)").starts_with("\\(1\\)"));
        let p = path_regex(&[PathBuf::from("/home/a b")], "q");
        assert_eq!(p, "^(/home/a b)/.*q");
        let p = path_regex(&[PathBuf::from("/h"), PathBuf::from("/m/n.x")], "q");
        assert_eq!(p, "^(/h|/m/n\\.x)/.*q");
    }

    #[test]
    fn recent_files_are_read_newest_first() {
        let text = r#"<?xml version="1.0"?><xbel>
<bookmark href="file:///home/u/old.txt" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z" visited="2026-01-01T00:00:00Z"></bookmark>
<bookmark href="file:///home/u/Fakturor/faktura-%C3%A5%C3%A4%C3%B6.pdf" added="2026-03-01T00:00:00Z" modified="2026-11-01T00:00:00Z" visited="2026-03-01T00:00:00Z"></bookmark>
<bookmark href="https://example.org/x" added="2026-12-01T00:00:00Z"></bookmark>
<bookmark href="file:///home/u/a&amp;b.txt" added="2026-02-01T00:00:00Z"></bookmark>
<bookmark href="file:///home/u/old.txt" added="2025-01-01T00:00:00Z"></bookmark>
</xbel>"#;
        assert_eq!(
            parse_recent(text),
            [
                "/home/u/Fakturor/faktura-åäö.pdf",
                "/home/u/a&b.txt",
                "/home/u/old.txt"
            ]
        );
        assert_eq!(percent_decode("a%2"), Some("a%2".into()));
        assert_eq!(percent_decode("%zz"), None);
    }

    #[test]
    fn rows_show_kind_name_and_short_folder() {
        let home = PathBuf::from("/home/u");
        let row = item("/home/u/Fakturor/faktura-åäö.pdf", false, Some(&home));
        assert_eq!(row.label(), "PDF · faktura-åäö.pdf");
        assert_eq!(row.detail, "~/Fakturor");
        assert_eq!(row.key, "file:/home/u/Fakturor/faktura-åäö.pdf");
        assert_eq!(item("/home/u/a", true, Some(&home)).group, "Folder");
        assert_eq!(item("/home/u/a.txt", false, Some(&home)).detail, "~");
        assert_eq!(item("/srv/a.txt", false, Some(&home)).detail, "/srv");
        assert_eq!(kind(".bashrc", false), "File");
        assert_eq!(kind("Bild.JPG", false), "Image");
        let long = "/home/u/one/two/three/four/five/six/seven/file.txt";
        let short = short_parent(long, Some(&home), 20);
        assert!(
            short.starts_with("~/…/") && short.ends_with("/seven"),
            "{short}"
        );
        assert!(short.chars().count() <= 20);
        let one = "/home/u/averyveryveryverylongfoldername/x";
        assert!(short_parent(one, Some(&home), 12).chars().count() <= 12);
        assert_eq!(parent_of("/home/u/a.txt"), "/home/u");
        assert_eq!(parent_of("/a"), "/");
    }

    #[test]
    fn the_prefix_is_a_slash() {
        assert_eq!(prefixed("/ faktura"), Some("faktura"));
        assert_eq!(prefixed("  /faktura  "), Some("faktura"));
        assert_eq!(prefixed("/"), Some(""));
        assert_eq!(prefixed("faktura"), None);
        assert_eq!(prefixed(":heart"), None);
        // Arithmetic never starts with a slash, so the calculator keeps 2/3.
        assert_eq!(prefixed("2/3"), None);
    }

    #[test]
    fn the_walker_and_fd_skip_the_same_things() {
        let dir = std::env::temp_dir().join(format!("launcher-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in [
            "Fakturor",
            ".hidden",
            "proj/node_modules/x",
            "proj/target",
            "proj/src",
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "Fakturor/faktura-åäö.pdf",
            ".hidden/faktura.pdf",
            "proj/node_modules/x/faktura.js",
            "proj/target/faktura.o",
            "proj/src/faktura.rs",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let mut c = cfg();
        c.roots = vec![dir.to_string_lossy().into_owned()];
        let roots = roots(&c);
        let w = Walk {
            entries: Mutex::new(Vec::new()),
            done: AtomicBool::new(false),
            truncated: AtomicBool::new(false),
        };
        walk_roots(&c, &roots, &w);
        assert!(w.done.load(Ordering::Relaxed));
        let t = Terms::new("faktura");
        let mut found: Vec<String> = w
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, rel, _)| t.score(rel).is_some())
            .map(|(_, rel, _)| rel.clone())
            .collect();
        found.sort();
        assert_eq!(found, ["fakturor/faktura-åäö.pdf", "proj/src/faktura.rs"]);
        if in_path("fdfind") || in_path("fd") {
            let program = if in_path("fdfind") { "fdfind" } else { "fd" };
            let never = AtomicBool::new(false);
            let (paths, done) = backend_paths(
                &Backend::Fd {
                    program: program.into(),
                    and: fd_has_and(&version_line(program)),
                },
                &c,
                &roots,
                &Terms::new("aao"),
                &never,
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
            assert!(done);
            assert!(
                paths
                    .iter()
                    .any(|p| p.ends_with("Fakturor/faktura-åäö.pdf")),
                "{paths:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
