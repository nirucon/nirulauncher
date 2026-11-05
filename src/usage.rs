//! Local launch history and its frecency score.
//!
//! `usage.tsv` keeps one `key<TAB>unix-seconds` row per launch, oldest first,
//! capped at [`MAX_ROWS`]. The format is the one 0.2.x wrote (one row per key
//! then), so either version reads the other's file: an older launcher keeps
//! the last row of each key, which is the newest launch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Launch rows kept on disk; older launches are dropped first.
pub const MAX_ROWS: usize = 500;

/// A launch loses half its weight every week, so last week's daily app ranks
/// above one used a lot two months ago.
const HALF_LIFE_SECS: f64 = 7.0 * 86_400.0;

#[derive(Clone, Debug, Default)]
pub struct Usage {
    /// Newest launch per key.
    last: HashMap<String, u64>,
    /// Sum of decayed launch weights per key.
    score: HashMap<String, f64>,
}

impl Usage {
    /// Build the score from rows at time `now`.
    pub fn from_rows<'a>(rows: impl IntoIterator<Item = (&'a str, u64)>, now: u64) -> Self {
        let mut usage = Self::default();
        for (key, stamp) in rows {
            let age = now.saturating_sub(stamp) as f64;
            *usage.score.entry(key.to_string()).or_default() += 0.5f64.powf(age / HALF_LIFE_SECS);
            let last = usage.last.entry(key.to_string()).or_default();
            *last = (*last).max(stamp);
        }
        usage
    }

    /// The frecency of `key`, scaled to an integer so it sorts exactly.
    pub fn score(&self, key: &str) -> u32 {
        self.score
            .get(key)
            .map(|value| (value * 1000.0).round().clamp(0.0, u32::MAX as f64) as u32)
            .unwrap_or(0)
    }

    pub fn last(&self, key: &str) -> Option<u64> {
        self.last.get(key).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    base.join("nirulauncher/usage.tsv")
}

/// Parse rows, skipping anything that is not `key<TAB>number`.
pub fn parse(text: &str) -> Vec<(String, u64)> {
    text.lines()
        .filter_map(|line| {
            let (key, stamp) = line.rsplit_once('\t')?;
            let stamp = stamp.trim().parse::<u64>().ok()?;
            (!key.is_empty()).then(|| (key.to_string(), stamp))
        })
        .collect()
}

pub fn load() -> Usage {
    load_from(&path(), now())
}

pub fn load_from(path: &Path, now: u64) -> Usage {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let rows = parse(&text);
    let skip = rows.len().saturating_sub(MAX_ROWS);
    Usage::from_rows(
        rows.iter()
            .skip(skip)
            .map(|(key, stamp)| (key.as_str(), *stamp)),
        now,
    )
}

/// Append one launch and keep the newest [`MAX_ROWS`] rows, oldest first.
pub fn record(key: &str) {
    record_at(&path(), key, now());
}

pub fn record_at(path: &Path, key: &str, now: u64) {
    if key.is_empty() || key.contains('\n') {
        return;
    }
    let mut rows = parse(&std::fs::read_to_string(path).unwrap_or_default());
    rows.push((key.to_string(), now));
    // 0.2.x wrote the rows newest first; a stable sort keeps launches in the
    // same second in their order.
    rows.sort_by_key(|(_, stamp)| *stamp);
    let skip = rows.len().saturating_sub(MAX_ROWS);
    let text: String = rows
        .iter()
        .skip(skip)
        .map(|(key, stamp)| format!("{key}\t{stamp}\n"))
        .collect();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension(format!("tsv.{}", std::process::id()));
    if std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    #[test]
    fn recent_and_frequent_launches_score_higher() {
        let now = 100 * DAY;
        let rows = [
            ("daily", now - DAY),
            ("daily", now - 2 * DAY),
            ("daily", now - 3 * DAY),
            ("old", now - 60 * DAY),
            ("old", now - 61 * DAY),
            ("old", now - 62 * DAY),
            ("old", now - 63 * DAY),
            ("once", now - DAY),
        ];
        let usage = Usage::from_rows(rows, now);
        assert!(usage.score("daily") > usage.score("once"));
        assert!(usage.score("once") > usage.score("old"), "weeks of decay");
        assert_eq!(usage.score("never"), 0);
        assert_eq!(usage.last("daily"), Some(now - DAY));
    }

    #[test]
    fn the_file_is_capped_and_reads_the_old_format() {
        let dir = std::env::temp_dir().join(format!("nl-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("usage.tsv");
        // 0.2.78 wrote one row per key, newest first.
        std::fs::write(&file, "b.desktop\t200\na.desktop\t100\nbroken line\n").unwrap();
        let usage = load_from(&file, 300);
        assert_eq!(usage.last("b.desktop"), Some(200));
        assert_eq!(usage.last("a.desktop"), Some(100));
        for i in 0..(MAX_ROWS + 20) {
            record_at(&file, "c.desktop", 1000 + i as u64);
        }
        let rows = parse(&std::fs::read_to_string(&file).unwrap());
        assert_eq!(rows.len(), MAX_ROWS);
        assert!(
            rows.windows(2).all(|pair| pair[0].1 <= pair[1].1),
            "oldest first"
        );
        assert_eq!(rows.last().unwrap().1, 1000 + MAX_ROWS as u64 + 19);
        // An older reader keeps the last row of a key: the newest launch.
        let mut old_reader = HashMap::new();
        for (key, stamp) in &rows {
            old_reader.insert(key.clone(), *stamp);
        }
        assert_eq!(old_reader["c.desktop"], 1000 + MAX_ROWS as u64 + 19);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
