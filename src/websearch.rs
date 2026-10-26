//! Web search from the launcher: one row per engine for the typed text.
//!
//! A source is an id, a label, a URL template with `{query}` in it and an
//! optional short alias. Adding one is a line in the built-in table or a
//! `[[search.sources]]` entry in `~/.config/nirulauncher/config.toml`; nothing
//! else has to change, so the list can grow without touching the keyboard
//! handling or the rendering.
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
/// first and one keystroke less away. The fourth field is a short alias for
/// the `alias query` form; it is only used when followed by whitespace, so a
/// normal word is never mistaken for a provider.
pub const BUILTIN: &[(&str, &str, &str, &str)] = &[
    (
        "duckduckgo",
        "DuckDuckGo",
        "https://duckduckgo.com/?q={query}",
        "ddg",
    ),
    (
        "brave",
        "Brave Search",
        "https://search.brave.com/search?q={query}",
        "brave",
    ),
    (
        "google",
        "Google",
        "https://www.google.com/search?q={query}",
        "g",
    ),
    (
        "wikipedia",
        "Wikipedia (English)",
        "https://en.wikipedia.org/w/index.php?search={query}",
        "w",
    ),
    (
        "wikipedia-sv",
        "Wikipedia (Swedish)",
        "https://sv.wikipedia.org/w/index.php?search={query}",
        "wsv",
    ),
    (
        "youtube",
        "YouTube",
        "https://www.youtube.com/results?search_query={query}",
        "yt",
    ),
    (
        "github",
        "GitHub",
        "https://github.com/search?q={query}",
        "gh",
    ),
    (
        "archwiki",
        "ArchWiki",
        "https://wiki.archlinux.org/index.php?search={query}",
        "aw",
    ),
    (
        "crates",
        "crates.io",
        "https://crates.io/search?q={query}",
        "crate",
    ),
    (
        "docsrs",
        "docs.rs",
        "https://docs.rs/releases/search?query={query}",
        "docs",
    ),
    (
        "chatgpt",
        "ChatGPT",
        "https://chatgpt.com/?q={query}",
        "gpt",
    ),
    (
        "gemini",
        "Gemini",
        "https://gemini.google.com/app?q={query}",
        "gem",
    ),
    ("grok", "Grok", "https://grok.com/?q={query}", "grok"),
    (
        "copilot",
        "Microsoft Copilot",
        "https://copilot.microsoft.com/?q={query}&sendquery=1",
        "cop",
    ),
    (
        "discogs",
        "Discogs",
        "https://www.discogs.com/search/?q={query}",
        "disc",
    ),
    (
        "musicbrainz",
        "MusicBrainz",
        "https://musicbrainz.org/search?query={query}",
        "mb",
    ),
    (
        "metal-archives",
        "Metal Archives",
        "https://www.metal-archives.com/search?searchString={query}&type=band_name",
        "ma",
    ),
];

/// One search provider: a stable id used in keys, a label, a URL template and
/// the short alias that selects it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub url: String,
    pub alias: String,
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

/// The engines with user additions, overrides, order and a default.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub sources: Vec<Source>,
    /// `(alias, source id)`, lower-case aliases, configuration wins.
    pub aliases: Vec<(String, String)>,
    /// The configured default id, when it names an enabled engine.
    pub default: Option<String>,
}

impl Catalog {
    /// The provider selected by a leading `alias ` prefix, with the rest of
    /// the text. Only an alias followed by whitespace counts, so an ordinary
    /// word, including a provider's full name, stays a normal query.
    pub fn prefixed<'a>(&'a self, text: &'a str) -> Option<(&'a Source, String)> {
        let (word, rest) = text.split_once(char::is_whitespace)?;
        let alias = word.trim().to_lowercase();
        if alias.is_empty() {
            return None;
        }
        let id = self
            .aliases
            .iter()
            .find(|(candidate, _)| candidate == &alias)
            .map(|(_, id)| id)?;
        let source = self.sources.iter().find(|source| &source.id == id)?;
        Some((source, rest.trim().to_string()))
    }

    /// The row selected when the field is empty: the configured default, or
    /// the first enabled engine.
    pub fn default_index(&self) -> usize {
        self.default
            .as_deref()
            .and_then(|id| self.sources.iter().position(|source| source.id == id))
            .unwrap_or(0)
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
        .map(|(id, label, url, alias)| Source {
            id: (*id).to_string(),
            label: (*label).to_string(),
            url: (*url).to_string(),
            alias: (*alias).to_string(),
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
        let alias = entry
            .alias
            .as_deref()
            .map(str::trim)
            .filter(|alias| !alias.is_empty() && !alias.contains(char::is_whitespace))
            .map(str::to_lowercase)
            .unwrap_or_else(|| {
                list.iter()
                    .find(|source| source.id == id)
                    .map(|source| source.alias.clone())
                    .unwrap_or_default()
            });
        let source = Source {
            id: id.clone(),
            label,
            url,
            alias,
        };
        list.retain(|existing| existing.id != id);
        // Keep a built-in's position so the familiar engines do not move when
        // one of them is customised. The count is how many earlier built-ins
        // are still in the list, which is the insertion point.
        let earlier_builtins = BUILTIN
            .iter()
            .position(|(builtin, _, _, _)| *builtin == id)
            .map(|position| {
                list.iter()
                    .filter(|source| {
                        BUILTIN
                            .iter()
                            .position(|(builtin, _, _, _)| *builtin == source.id)
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

/// The final provider order and aliases from the launcher configuration.
///
/// `order` names enabled ids; listed ids come first in that order, and the
/// rest keep their built-in or append order. Unknown ids are ignored. An
/// empty default falls back to the first enabled engine.
pub fn catalog(
    extra: &[ConfigSource],
    order: &[String],
    default: Option<&str>,
    aliases: &[(String, String)],
) -> Catalog {
    let base = sources(extra);
    let mut list: Vec<Source> = Vec::with_capacity(base.len());
    for id in order {
        let id = id.trim().to_lowercase();
        if let Some(source) = base.iter().find(|source| source.id == id) {
            if !list.iter().any(|existing| existing.id == source.id) {
                list.push(source.clone());
            }
        }
    }
    for source in base {
        if !list.iter().any(|existing| existing.id == source.id) {
            list.push(source);
        }
    }

    let mut alias_map: Vec<(String, String)> = Vec::new();
    let mut set_alias = |alias: &str, id: &str| {
        let alias = alias.trim().to_lowercase();
        if alias.is_empty()
            || alias.contains(char::is_whitespace)
            || !list.iter().any(|source| source.id == id)
        {
            return;
        }
        alias_map.retain(|(existing, _)| existing != &alias);
        alias_map.push((alias, id.to_string()));
    };
    for source in &list {
        set_alias(&source.alias, &source.id);
    }
    // A configured alias overrides a built-in one with the same spelling.
    for (alias, id) in aliases {
        set_alias(alias, &id.trim().to_lowercase());
    }

    let default = default
        .map(|id| id.trim().to_lowercase())
        .filter(|id| list.iter().any(|source| &source.id == id));
    Catalog {
        sources: list,
        aliases: alias_map,
        default,
    }
}

/// A `[[search.sources]]` entry as written in the configuration file.
#[derive(Clone, Debug, Default)]
pub struct ConfigSource {
    pub id: String,
    pub label: Option<String>,
    pub url: String,
    pub alias: Option<String>,
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
        // The key names the engine only. It is what favourites and the usage
        // file store, and both are plain text on disk: the words a person
        // searches with must not be written there. Ranking by engine still
        // works, because that is a preference anyway.
        key: format!("search:{}", source.id),
        group: "Search".into(),
        name: format!("{}: {text}", source.label),
        detail: source.host(),
        keywords: format!("{} {}", source.label, source.alias),
        favorite: false,
        target: Target::Spawn {
            argv: browser_command(&source.label, &url),
            terminal: false,
            path: None,
        },
    })
}

/// A provider row shown before anything has been typed. It never opens the
/// browser; Enter on an empty field keeps the launcher open.
pub fn provider_item(source: &Source) -> crate::Item {
    crate::Item {
        key: format!("search:{}", source.id),
        group: "Search".into(),
        name: source.label.clone(),
        detail: source.host(),
        keywords: source.alias.clone(),
        favorite: false,
        target: Target::Info,
    }
}

/// A row that opens the typed address itself.
pub fn address_item(url: &str) -> crate::Item {
    crate::Item {
        key: "search:address".into(),
        group: String::new(),
        name: "Open address".into(),
        detail: url.to_string(),
        keywords: "url link website".into(),
        favorite: false,
        target: Target::Spawn {
            argv: browser_command("Address", url),
            terminal: false,
            path: None,
        },
    }
}

/// The address itself when the text is an explicit HTTP(S) URL.
///
/// Only `http://` and `https://` are accepted: a search field must never turn
/// `file:` or a bare command into a local action.
pub fn web_address(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || host.starts_with(':') || host.starts_with('.') {
        return None;
    }
    if !host.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    Some(text.to_string())
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

    fn source(id: &str, alias: &str) -> Source {
        Source {
            id: id.into(),
            label: id.into(),
            url: format!("https://{id}.example/?q={{query}}"),
            alias: alias.into(),
        }
    }

    #[test]
    fn a_builtin_engine_has_its_query() {
        let source = &Source {
            id: "duckduckgo".into(),
            label: "DuckDuckGo".into(),
            url: "https://duckduckgo.com/?q={query}".into(),
            alias: "ddg".into(),
        };
        assert_eq!(
            source.url_for("black metal 1994").as_deref(),
            Some("https://duckduckgo.com/?q=black+metal+1994")
        );
        assert_eq!(source.host(), "duckduckgo.com");
    }

    /// Every default engine must carry the words somewhere, or the row would
    /// open a page that ignores what was typed.
    #[test]
    fn every_builtin_engine_carries_the_query() {
        for source in sources(&[]) {
            let url = source.url_for("test").expect("a built-in engine opens");
            assert!(url.contains("test"), "{} drops the query: {url}", source.id);
            assert!(url.starts_with("https://"), "{}: {url}", source.id);
        }
    }

    /// The key is written to the usage and favourites files in plain text, so
    /// it may not contain what the user searched for.
    #[test]
    fn the_row_key_does_not_contain_the_query() {
        for source in sources(&[]) {
            let item = item(&source, "my private phrase").expect("a row per engine");
            assert_eq!(item.key, format!("search:{}", source.id));
            assert!(
                !item.key.contains("private"),
                "the query leaked into a stored key: {}",
                item.key
            );
        }
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
        // The provider itself is still listed before anything is typed.
        let row = provider_item(&sources[0]);
        assert_eq!(row.target_kind(), "info");
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
            "wikipedia-sv",
            "youtube",
            "github",
            "archwiki",
            "crates",
            "docsrs",
            "discogs",
            "musicbrainz",
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
            alias: None,
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
                alias: Some("sx".into()),
                enabled: true,
            },
            ConfigSource {
                id: "grok".into(),
                label: None,
                url: String::new(),
                alias: None,
                enabled: false,
            },
        ]);
        assert_eq!(list.last().unwrap().label, "Searx");
        assert_eq!(list.last().unwrap().alias, "sx");
        assert!(list.iter().all(|source| source.id != "grok"));
    }

    #[test]
    fn a_template_without_a_placeholder_still_searches() {
        let source = Source {
            id: "plain".into(),
            label: "Plain".into(),
            url: "https://example.com/search".into(),
            alias: String::new(),
        };
        assert_eq!(
            source.url_for("rust").as_deref(),
            Some("https://example.com/search?q=rust")
        );
        let unsafe_url = Source {
            id: "bad".into(),
            label: "Bad".into(),
            url: "file:///etc/passwd?q={query}".into(),
            alias: String::new(),
        };
        assert!(unsafe_url.url_for("x").is_none());
    }

    #[test]
    fn the_catalog_applies_order_and_default() {
        let catalog = catalog(
            &[],
            &["github".into(), "duckduckgo".into()],
            Some("github"),
            &[],
        );
        assert_eq!(catalog.sources[0].id, "github");
        assert_eq!(catalog.sources[1].id, "duckduckgo");
        assert_eq!(catalog.default_index(), 0);
    }

    #[test]
    fn a_missing_default_falls_back_to_the_first_engine() {
        let catalog = catalog(&[], &[], Some("not-there"), &[]);
        assert_eq!(catalog.default, None);
        assert_eq!(catalog.default_index(), 0);
    }

    #[test]
    fn aliases_need_whitespace_and_configuration_wins() {
        let catalog = catalog(
            &[],
            &[],
            None,
            &[
                ("gh".into(), "wikipedia".into()),
                ("x".into(), "github".into()),
            ],
        );
        let (source, rest) = catalog.prefixed("gh niruwm").unwrap();
        assert_eq!(source.id, "wikipedia", "a configured alias overrides");
        assert_eq!(rest, "niruwm");
        assert!(catalog.prefixed("github niruwm").is_none());
        assert!(catalog.prefixed("gh").is_none());
        assert!(catalog.prefixed("ordinary words").is_none());
        assert_eq!(catalog.prefixed("x  two words ").unwrap().1, "two words");
    }

    #[test]
    fn only_explicit_http_addresses_are_addresses() {
        assert_eq!(
            web_address("https://example.com/a?q=1#x").as_deref(),
            Some("https://example.com/a?q=1#x")
        );
        assert_eq!(
            web_address("http://localhost:8080").as_deref(),
            Some("http://localhost:8080")
        );
        for text in [
            "example.com",
            "file:///etc/passwd",
            "https://",
            "https:// two",
            "javascript:alert(1)",
        ] {
            assert!(web_address(text).is_none(), "{text} is not an address");
        }
    }

    #[test]
    fn a_provider_row_without_a_query_is_inert() {
        let row = provider_item(&source("github", "gh"));
        assert_eq!(row.name, "github");
        assert_eq!(row.detail, "github.example");
        assert_eq!(row.target_kind(), "info");
        let row = item(&source("github", "gh"), "x").unwrap();
        assert_eq!(row.target_kind(), "spawn");
    }
}
