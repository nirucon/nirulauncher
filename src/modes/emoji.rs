//! The emoji and symbol picker (`--emoji`, Super+.), and `:name` in the
//! unified launcher.
//!
//! The data is compiled in from Unicode emoji-test.txt and CLDR annotations
//! (English and Swedish names and keywords), see `tools/gen-emoji.py`. Rows
//! keep the base emoji in their key; the skin tone chosen with Tab is applied
//! when a row is drawn or used, so recents and favourites stay one row.

use crate::*;

const DATA: &str = include_str!("../../data/emoji.tsv");

/// The group of the typographic symbols, drawn with the text font.
pub(crate) const TEXT_SYMBOLS: &str = "Text symbols";
pub(crate) const KEY_PREFIX: &str = "emoji:";
const RECENT_MAX: usize = 24;

pub(crate) struct Emoji {
    pub(crate) group: &'static str,
    pub(crate) chars: &'static str,
    pub(crate) name: &'static str,
    pub(crate) name_sv: &'static str,
    pub(crate) keywords: &'static str,
    /// Light to dark skin tone variants, or empty.
    pub(crate) tones: Vec<&'static str>,
}

pub(crate) fn data() -> &'static [Emoji] {
    static DATA_ROWS: std::sync::OnceLock<Vec<Emoji>> = std::sync::OnceLock::new();
    DATA_ROWS.get_or_init(|| {
        DATA.lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let group = f.next()?;
                let chars = f.next()?;
                let name = f.next()?;
                let name_sv = f.next().unwrap_or("");
                let keywords = f.next().unwrap_or("");
                let tones: Vec<&str> = f
                    .next()
                    .unwrap_or("")
                    .split(' ')
                    .filter(|t| !t.is_empty())
                    .collect();
                (!chars.is_empty()).then_some(Emoji {
                    group,
                    chars,
                    name,
                    name_sv,
                    keywords,
                    tones: if tones.len() == 5 { tones } else { Vec::new() },
                })
            })
            .collect()
    })
}

fn by_chars(chars: &str) -> Option<&'static Emoji> {
    static INDEX: std::sync::OnceLock<std::collections::HashMap<&'static str, usize>> =
        std::sync::OnceLock::new();
    let index = INDEX.get_or_init(|| {
        data()
            .iter()
            .enumerate()
            .map(|(i, e)| (e.chars, i))
            .collect()
    });
    index.get(chars).map(|&i| &data()[i])
}

/// The characters a row inserts with skin tone `tone` (0 = none, 1-5).
pub(crate) fn with_tone(key: &str, tone: u8) -> Option<String> {
    let chars = key.strip_prefix(KEY_PREFIX)?;
    let e = by_chars(chars)?;
    Some(match tone {
        1..=5 if !e.tones.is_empty() => e.tones[usize::from(tone - 1)].to_string(),
        _ => e.chars.to_string(),
    })
}

pub(crate) fn is_text_symbol(key: &str) -> bool {
    key.strip_prefix(KEY_PREFIX)
        .and_then(by_chars)
        .is_some_and(|e| e.group == TEXT_SYMBOLS)
}

#[cfg(test)]
pub(crate) fn has_tones(key: &str) -> bool {
    key.strip_prefix(KEY_PREFIX)
        .and_then(by_chars)
        .is_some_and(|e| !e.tones.is_empty())
}

fn item(e: &Emoji) -> Item {
    Item {
        key: format!("{KEY_PREFIX}{}", e.chars),
        group: e.group.into(),
        name: e.name.into(),
        detail: e.name_sv.into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }
}

fn state_dir() -> std::path::PathBuf {
    usage::path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default()
}

fn recent_path() -> std::path::PathBuf {
    state_dir().join("emoji-recent.txt")
}

fn tone_path() -> std::path::PathBuf {
    state_dir().join("emoji-tone")
}

pub(crate) fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| by_chars(l).is_some())
        .take(RECENT_MAX)
        .map(str::to_string)
        .collect()
}

/// Put `key`'s base emoji first in the recent list.
pub(crate) fn record_recent(key: &str) {
    let Some(chars) = key.strip_prefix(KEY_PREFIX) else {
        return;
    };
    let mut list = load_recent();
    list.retain(|c| c != chars);
    list.insert(0, chars.to_string());
    list.truncate(RECENT_MAX);
    let path = recent_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, list.join("\n") + "\n");
}

pub(crate) fn load_tone() -> u8 {
    std::fs::read_to_string(tone_path())
        .ok()
        .and_then(|t| t.trim().parse::<u8>().ok())
        .filter(|t| *t <= 5)
        .unwrap_or(0)
}

pub(crate) fn save_tone(tone: u8) {
    let path = tone_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{tone}\n"));
}

pub(crate) fn tone_name(tone: u8) -> &'static str {
    match tone {
        1 => "light",
        2 => "medium-light",
        3 => "medium",
        4 => "medium-dark",
        5 => "dark",
        _ => "default (yellow)",
    }
}

/// Every row, recently used first, then in Unicode order.
pub(crate) fn emoji_items() -> Vec<Item> {
    let recent = load_recent();
    let mut items: Vec<Item> = recent
        .iter()
        .filter_map(|c| by_chars(c))
        .map(item)
        .collect();
    items.extend(
        data()
            .iter()
            .filter(|e| !recent.iter().any(|r| r == e.chars))
            .map(item),
    );
    items
}

fn word_rank(text: &str, term: &str) -> Option<u8> {
    let text = text.to_lowercase();
    if text == term {
        return Some(0);
    }
    if text.starts_with(term) {
        return Some(1);
    }
    if text
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(term))
    {
        return Some(2);
    }
    text.contains(term).then_some(5)
}

/// How well one emoji matches every term (lower is better).
fn score(e: &Emoji, terms: &[String]) -> Option<u32> {
    let mut total = 0u32;
    for term in terms {
        let best = [
            word_rank(e.name, term),
            word_rank(e.name_sv, term).map(|r| r + 1),
            e.keywords
                .split('|')
                .filter_map(|k| word_rank(k, term))
                .min()
                .map(|r| r + 2),
            word_rank(e.group, term).map(|r| r + 6),
        ]
        .into_iter()
        .flatten()
        .min()?;
        total += u32::from(best);
    }
    Some(total)
}

fn terms(query: &str) -> Vec<String> {
    query
        .trim()
        .trim_start_matches(':')
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Order the picker's rows for `query`: the given order (recents first) when
/// it is empty, otherwise by match quality, then recents, then Unicode order.
pub(crate) fn emoji_ranked(items: &[Item], query: &str) -> Vec<usize> {
    let terms = terms(query);
    if terms.is_empty() {
        return (0..items.len()).collect();
    }
    let mut hits: Vec<(u8, u32, usize, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let e = by_chars(it.key.strip_prefix(KEY_PREFIX)?)?;
            let phrase = terms.join(" ");
            // The whole phrase as the name beats words found anywhere.
            let bonus = match word_rank(e.name, &phrase).or(word_rank(e.name_sv, &phrase)) {
                Some(0) => 0,
                Some(1) => 1,
                _ => 2,
            };
            Some((bonus, score(e, &terms)?, e.name.len(), i))
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|h| h.3).collect()
}

/// `:name` in the unified launcher: the best few matches as rows.
pub(crate) fn colon_rows(query: &str) -> Vec<Item> {
    let q = query.trim_start();
    if !q.starts_with(':') || q.trim_start_matches(':').trim().is_empty() {
        return Vec::new();
    }
    let all: Vec<Item> = emoji_items();
    emoji_ranked(&all, q)
        .into_iter()
        .take(8)
        .map(|i| all[i].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_has_emoji_and_symbols_with_names() {
        let d = data();
        assert!(d.len() > 1900, "{}", d.len());
        assert!(d
            .iter()
            .any(|e| e.chars == "😀" && e.name == "grinning face"));
        assert!(d.iter().any(|e| e.chars == "©" && e.group == TEXT_SYMBOLS));
        assert!(d.iter().any(|e| e.chars == "€"));
        assert!(d.iter().any(|e| e.chars == "→"));
        assert!(d.iter().any(|e| e.chars == "°"));
        assert!(d.iter().any(|e| e.chars == "”"));
        let swedish = d.iter().filter(|e| !e.name_sv.is_empty()).count();
        assert!(swedish * 100 / d.len() > 98, "Swedish names: {swedish}");
        // Components are not rows: tones come through Tab.
        assert!(!d.iter().any(|e| e.chars == "\u{1F3FB}"));
    }

    #[test]
    fn search_finds_english_swedish_and_keywords() {
        let items = emoji_items();
        let first = |q: &str| items[emoji_ranked(&items, q)[0]].key.clone();
        assert_eq!(first("grinning face"), "emoji:😀");
        assert_eq!(first("thumbs up"), "emoji:👍");
        assert!(first(":copyright").starts_with("emoji:©"));
        assert_eq!(first("euro"), "emoji:€");
        assert!(emoji_ranked(&items, "hjärta")
            .iter()
            .take(5)
            .any(|&i| items[i].key == "emoji:❤️"));
        assert!(emoji_ranked(&items, "gåsögon").len() >= 2);
        assert!(emoji_ranked(&items, "zzzqqq").is_empty());
    }

    #[test]
    fn skin_tones_apply_to_rows_that_have_them() {
        assert_eq!(with_tone("emoji:👋", 0).unwrap(), "👋");
        assert_eq!(with_tone("emoji:👋", 3).unwrap(), "👋\u{1F3FD}");
        assert_eq!(with_tone("emoji:😀", 3).unwrap(), "😀");
        assert!(has_tones("emoji:👍"));
        assert!(!has_tones("emoji:😀"));
        assert!(is_text_symbol("emoji:™"));
    }

    #[test]
    fn colon_queries_offer_a_few_rows() {
        let rows = colon_rows(":smile");
        assert!(!rows.is_empty() && rows.len() <= 8);
        assert!(rows.iter().all(|r| r.key.starts_with(KEY_PREFIX)));
        assert!(colon_rows(":").is_empty());
        assert!(colon_rows("smile").is_empty());
    }
}
