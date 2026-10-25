//! Web search from the launcher: one row per engine for the typed text.
//!
//! A source is an id, a label and a URL template with `{query}` in it. Adding
//! one is a line in the built-in table or a `[[search.sources]]` entry in
//! `~/.config/nirulauncher/config.toml`; nothing else has to change, so the
//! list can grow without touching the keyboard handling or the rendering.
//!
//! The launcher never talks to the network itself. It opens an address in the
//! browser, which keeps the query out of any log of its own and means a search
//! costs exactly the same as opening any other link.

use crate::{Config, Target};
use std::collections::HashSet;

/// The engines NIRUX offers without configuration.
///
/// `{query}` is replaced by the percent-encoded text; `{}` is accepted too.
/// The order is the order the rows appear in, so the most common engines are
/// first and one keystroke less away.
pub const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "duckduckgo",
        "DuckDuckGo",
        "https://duckduckgo.com/?q={query}",
    ),
    (
        "brave",
        "Brave Search",
        "https://search.brave.com/search?q={query}",
    ),
    (
        "google",
        "Google",
        "https://www.google.com/search?q={query}",
    ),
    (
        "wikipedia",
        "Wikipedia (English)",
        "https://en.wikipedia.org/w/index.php?search={query}",
    ),
    (
        "youtube",
        "YouTube",
        "https://www.youtube.com/results?search_query={query}",
    ),
    ("chatgpt", "ChatGPT", "https://chatgpt.com/?q={query}"),
    (
        "gemini",
        "Gemini",
        "https://gemini.google.com/app?q={query}",
    ),
    ("grok", "Grok", "https://grok.com/?q={query}"),
    (
        "copilot",
        "Microsoft Copilot",
        "https://copilot.microsoft.com/?q={query}",
    ),
    (
        "metal-archives",
        "Metal Archives",
        "https://www.metal-archives.com/search?searchString={query}&type=band_name",
    ),
];

/// One search provider: a stable id used in keys, a label, a URL template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub url: String,
}

impl Source {
    /// The address for this query, or `None` when the template is unusable.
    pub fn url_for(&self, query: &str) -> Option<String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return None;
        }
        let encoded = percent_encode(trimmed);
        let mut url = None;
        for marker in ["{query}", "{}"] {
            if self.url.contains(marker) {
                url = Some(self.url.replace(marker, &encoded));
                break;
            }
        }
        // A template without a placeholder gets the query appended, so a
        // hand-written entry with a plain address still works.
        let url = url.unwrap_or_else(|| format!("{}?q={}", self.url, encoded));
        // Only web addresses are opened: a mistyped template must not become a
        // way to start a local program.
        if url.starts_with("https://") || url.starts_with("http://") {
            Some(url)
        } else {
            None
        }
    }

    /// The host shown in the row, for example `search.brave.com`.
    pub fn host(&self) -> String {
        self.url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.url)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .trim_start_matches("www.")
            .to_string()
    }
}

/// The built-in engines with any `[[search.sources]]` additions or overrides.
///
/// An entry with the same id as a built-in replaces it in place, keeping the
/// familiar order; a new id is appended. `enabled = false` removes an engine,
/// and an empty `[[search.sources]]` table that only turns some off leaves the
/// rest as they are.
pub fn sources(extra: &[ConfigSource]) -> Vec<Source> {
    let mut list: Vec<Source> = BUILTIN
        .iter()
        .map(|(id, label, url)| Source {
            id: (*id).to_string(),
            label: (*label).to_string(),
            url: (*url).to_string(),
        })
        .collect();
    let mut removed: HashSet<String> = HashSet::new();
    for entry in extra {
        let id = entry.id.trim().to_lowercase();
        if id.is_empty() || !entry.enabled {
            if !id.is_empty() {
                removed.insert(id.clone());
            }
            continue;
        }
        let url = entry.url.trim().to_string();
        let label = entry
            .label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                list.iter()
                    .find(|source| source.id == id)
                    .map(|source| source.label.clone())
                    .unwrap_or_else(|| {
                        // `metal-archives` -> `Metal Archives`
                        id.split(['-', '_'])
                            .map(|word| {
                                let mut chars = word.chars();
                                match chars.next() {
                                    Some(first) => {
                                        format!("{}{}", first.to_uppercase(), chars.as_str())
                                    }
                                    None => String::new(),
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
            });
        if url.is_empty() {
            // No address: the entry can still label an engine, nothing more.
            continue;
        }
        let source = Source {
            id: id.clone(),
            label,
            url,
        };
        list.retain(|existing| existing.id != id);
        // Keep a built-in's position so the familiar engines do not move when
        // one of them is customised. The count is how many earlier built-ins
        // are still in the list, which is the insertion point.
        let earlier_builtins = BUILTIN
            .iter()
            .position(|(builtin, _, _)| *builtin == id)
            .map(|position| {
                list.iter()
                    .filter(|source| {
                        BUILTIN
                            .iter()
                            .position(|(builtin, _, _)| *builtin == source.id)
                            .is_some_and(|earlier| earlier < position)
                    })
                    .count()
            });
        match earlier_builtins {
            Some(index) => list.insert(index.min(list.len()), source),
            None => list.push(source),
        }
    }
    list.retain(|source| !removed.contains(&source.id));
    list
}

/// A `[[search.sources]]` entry as written in the configuration file.
#[derive(Clone, Debug, Default)]
pub struct ConfigSource {
    pub id: String,
    pub label: Option<String>,
    pub url: String,
    pub enabled: bool,
}

/// Percent-encode everything that is not unreserved in a query, with `+` for
/// spaces, which every search engine accepts.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A row that opens one engine's results for `query`.
pub fn item(source: &Source, query: &str) -> Option<crate::Item> {
    let url = source.url_for(query)?;
    let text = query.trim().to_string();
    Some(crate::Item {
        key: format!("search:{}:{}", source.id, text),
        group: "Search".into(),
        name: format!("{}: {text}", source.label),
        detail: source.host(),
        favorite: false,
        target: Target::Spawn {
            argv: browser_command(&source.label, &url),
            terminal: false,
            path: None,
        },
    })
}

/// How NIRUX opens a link.
///
/// `browser` from the launcher configuration first, because a NIRUX session
/// already knows its browser; then the same key NIRUWM uses; then `$BROWSER`;
/// and finally `xdg-open`, which follows the desktop's own default.
pub fn browser_command(label: &str, url: &str) -> Vec<String> {
    if let Some(argv) = configured_browser() {
        let mut argv = argv;
        argv.push(url.to_string());
        return argv;
    }
    if let Ok(browser) = std::env::var("BROWSER") {
        if let Some(program) = browser.split(':').find(|part| !part.trim().is_empty()) {
            return vec![program.trim().to_string(), url.to_string()];
        }
    }
    let _ = label;
    vec!["xdg-open".to_string(), url.to_string()]
}

/// `browser = [...]` from the launcher config, then NIRUWM's `config.toml`.
fn configured_browser() -> Option<Vec<String>> {
    let mut argv = Config::load()
        .ok()
        .map(|cfg| cfg.browser)
        .unwrap_or_default();
    if argv.is_empty() {
        argv = niruwm_browser();
    }
    // An empty first word would become the program name; drop it.
    argv.retain(|part| !part.trim().is_empty());
    (!argv.is_empty()).then_some(argv)
}

fn niruwm_browser() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Vec::new();
    };
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let Some(text) = std::fs::read_to_string(dir.join("niruwm/config.toml")).ok() else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    value
        .get("browser")
        .and_then(toml::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_builtin_engine_has_its_query() {
        let source = &Source {
            id: "duckduckgo".into(),
            label: "DuckDuckGo".into(),
            url: "https://duckduckgo.com/?q={query}".into(),
        };
        assert_eq!(
            source.url_for("black metal 1994").as_deref(),
            Some("https://duckduckgo.com/?q=black+metal+1994")
        );
        assert_eq!(source.host(), "duckduckgo.com");
    }

    #[test]
    fn special_characters_are_encoded() {
        assert_eq!(percent_encode("a&b=c?d/e#f"), "a%26b%3Dc%3Fd%2Fe%23f");
        assert_eq!(percent_encode("å ä ö"), "%C3%A5+%C3%A4+%C3%B6");
        assert_eq!(percent_encode("Metal: Archives"), "Metal%3A+Archives");
    }

    #[test]
    fn an_empty_query_has_no_row() {
        let sources = sources(&[]);
        assert!(item(&sources[0], "   ").is_none());
    }

    #[test]
    fn every_builtin_engine_is_complete() {
        let list = sources(&[]);
        let ids: HashSet<&str> = list.iter().map(|source| source.id.as_str()).collect();
        for expected in [
            "duckduckgo",
            "brave",
            "google",
            "wikipedia",
            "youtube",
            "chatgpt",
            "gemini",
            "grok",
            "copilot",
            "metal-archives",
        ] {
            assert!(ids.contains(expected), "missing {expected}");
        }
        for source in &list {
            assert!(
                source.url_for("x").is_some(),
                "{} has no usable url",
                source.id
            );
        }
    }

    #[test]
    fn a_configured_engine_replaces_a_builtin_in_place() {
        let list = sources(&[ConfigSource {
            id: "google".into(),
            label: Some("Google (no tracking)".into()),
            url: "https://www.google.com/search?q={query}&pws=0".into(),
            enabled: true,
        }]);
        assert_eq!(list.iter().position(|s| s.id == "google"), Some(2));
        assert_eq!(list[2].label, "Google (no tracking)");
        assert!(list[2].url_for("x").unwrap().ends_with("&pws=0"));
    }

    #[test]
    fn a_new_engine_is_appended_and_one_can_be_turned_off() {
        let list = sources(&[
            ConfigSource {
                id: "searx".into(),
                label: None,
                url: "https://searx.example/search?q={query}".into(),
                enabled: true,
            },
            ConfigSource {
                id: "grok".into(),
                label: None,
                url: String::new(),
                enabled: false,
            },
        ]);
        assert_eq!(list.last().unwrap().label, "Searx");
        assert!(list.iter().all(|source| source.id != "grok"));
    }

    #[test]
    fn a_template_without_a_placeholder_still_searches() {
        let source = Source {
            id: "plain".into(),
            label: "Plain".into(),
            url: "https://example.com/search".into(),
        };
        assert_eq!(
            source.url_for("rust").as_deref(),
            Some("https://example.com/search?q=rust")
        );
        let unsafe_url = Source {
            id: "bad".into(),
            label: "Bad".into(),
            url: "file:///etc/passwd?q={query}".into(),
        };
        assert!(unsafe_url.url_for("x").is_none());
    }
}
