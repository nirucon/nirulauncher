//! Ranking: matching rows against a query, favourites and launch history.

use crate::*;

pub(crate) fn usage_path() -> std::path::PathBuf {
    usage::path()
}

/// The history used for ranking, or none when `rank_by_usage = false`.
pub(crate) fn load_usage(cfg: &Config) -> usage::Usage {
    if cfg.rank_by_usage {
        usage::load()
    } else {
        usage::Usage::default()
    }
}

pub(crate) fn record_usage(key: &str) {
    usage::record(key);
}

pub(crate) fn favorites_path() -> std::path::PathBuf {
    usage_path().with_file_name("favorites.tsv")
}

pub(crate) fn removed_favorites_path() -> std::path::PathBuf {
    usage_path().with_file_name("favorites-removed.tsv")
}

pub(crate) fn load_removed_favorites() -> Vec<String> {
    std::fs::read_to_string(removed_favorites_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn save_removed_favorites(keys: &[String]) {
    let path = removed_favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text: String = keys.iter().map(|key| format!("{key}\n")).collect();
    let _ = std::fs::write(path, text);
}

pub(crate) fn load_runtime_favorites() -> Vec<String> {
    std::fs::read_to_string(favorites_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(5)
        .map(str::to_string)
        .collect()
}

pub(crate) fn save_runtime_favorites(keys: &[String]) {
    let path = favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text: String = keys.iter().take(5).map(|key| format!("{key}\n")).collect();
    let _ = std::fs::write(path, text);
}

/// True when every character of `needle` appears in `haystack` in order.
pub(crate) fn subsequence(haystack: &str, needle: &str) -> bool {
    let mut wanted = needle.chars();
    let Some(mut next) = wanted.next() else {
        return true;
    };
    for ch in haystack.chars() {
        if ch == next {
            match wanted.next() {
                Some(ch) => next = ch,
                None => return true,
            }
        }
    }
    false
}

/// How strongly one word matches an item; lower is better.
///
/// The order is deliberate: exact and prefix matches stay in front of a
/// multiword hit, metadata comes after the visible name, and fuzzy matching is
/// last so a typo never hides an obvious result.
pub(crate) fn term_rank(
    term: &str,
    name: &str,
    label: &str,
    group: &str,
    detail: &str,
    keywords: &str,
) -> Option<u8> {
    if name == term {
        Some(0)
    } else if name.starts_with(term) || label.starts_with(term) {
        Some(1)
    } else if name
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word == term)
    {
        Some(2)
    } else if name.contains(term) || label.contains(term) {
        Some(3)
    } else if keywords.contains(term) || group.contains(term) || detail.contains(term) {
        Some(4)
    } else if term.chars().count() >= 3 && (subsequence(name, term) || subsequence(label, term)) {
        Some(5)
    } else {
        None
    }
}

/// The lowercased text of one row that a query is matched against, plus the
/// same text with diacritics folded (`é` → `e`, `å`/`ä` → `a`).
pub(crate) struct SearchFields {
    pub(crate) exact: [String; 5],
    pub(crate) folded: [String; 5],
}

impl SearchFields {
    pub(crate) fn new(item: &Item) -> Self {
        let exact = [
            item.name.to_lowercase(),
            item.label().to_lowercase(),
            item.group.to_lowercase(),
            item.detail.to_lowercase(),
            item.keywords.to_lowercase(),
        ];
        let folded = exact.clone().map(|text| compose::fold(&text));
        Self { exact, folded }
    }

    /// The rank of one lowercase term; lower is better. A match on the exact
    /// text ranks ahead of the same kind of match that needed folding, so
    /// `é` finds `é` first, while `e` still finds `é`.
    pub(crate) fn rank(&self, term: &str) -> Option<u8> {
        let [name, label, group, detail, keywords] = &self.exact;
        let exact = term_rank(term, name, label, group, detail, keywords).map(|rank| rank * 2);
        let term = compose::fold(term);
        let [name, label, group, detail, keywords] = &self.folded;
        let folded =
            term_rank(&term, name, label, group, detail, keywords).map(|rank| rank * 2 + 1);
        match (exact, folded) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// The part of a row a query matched, for drawing.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Highlight {
    /// Character positions in `Item::label()` to underline.
    pub(crate) positions: Vec<usize>,
    /// A keyword or detail word, when the visible label did not match.
    pub(crate) keyword: Option<String>,
}

/// Where each query term matched a row: a substring of the label (exact,
/// then with diacritics folded), else its letters in order, else a word from
/// the keywords or detail.
pub(crate) fn highlight(item: &Item, query: &str) -> Highlight {
    // One lowercase character per label character, so positions line up.
    let lower: Vec<char> = item
        .label()
        .chars()
        .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
        .collect();
    let folded: Vec<char> = lower.iter().map(|&ch| compose::fold_char(ch)).collect();
    let mut found = Highlight::default();
    for term in query.to_lowercase().split_whitespace() {
        let term: Vec<char> = term.chars().collect();
        let term_folded: Vec<char> = term.iter().map(|&ch| compose::fold_char(ch)).collect();
        let at = |haystack: &[char], needle: &[char]| {
            (needle.len() <= haystack.len())
                .then(|| {
                    // Prefer a match at the start of a word.
                    let starts: Vec<usize> = (0..=haystack.len() - needle.len())
                        .filter(|&start| haystack[start..start + needle.len()] == *needle)
                        .collect();
                    starts
                        .iter()
                        .copied()
                        .find(|&start| start == 0 || !haystack[start - 1].is_alphanumeric())
                        .or_else(|| starts.first().copied())
                })
                .flatten()
        };
        if let Some(start) = at(&lower, &term).or_else(|| at(&folded, &term_folded)) {
            found.positions.extend(start..start + term.len());
            continue;
        }
        if term.len() >= 3 {
            let mut positions = Vec::new();
            let mut wanted = term_folded.iter().peekable();
            for (index, ch) in folded.iter().enumerate() {
                if wanted.peek() == Some(&ch) {
                    positions.push(index);
                    wanted.next();
                }
            }
            if wanted.peek().is_none() {
                found.positions.extend(positions);
                continue;
            }
        }
        if found.keyword.is_none() {
            let wanted: String = term_folded.iter().collect();
            found.keyword = item
                .keywords
                .split_whitespace()
                .chain(item.detail.split_whitespace())
                .find(|word| compose::fold(&word.to_lowercase()).contains(&wanted))
                .map(|word| {
                    word.trim_matches(|ch: char| !ch.is_alphanumeric())
                        .to_string()
                })
                .filter(|word| !word.is_empty());
        }
    }
    found.positions.sort_unstable();
    found.positions.dedup();
    found
}

/// The ordered rows for a query, and which of them form the "Recent" block.
pub(crate) struct Ranked {
    pub(crate) order: Vec<usize>,
    pub(crate) recent: std::ops::Range<usize>,
}

/// How many recent rows an empty query shows after the favourites.
pub(crate) const RECENT_ROWS: usize = 5;

#[cfg(test)]
pub(crate) fn filtered(items: &[Item], query: &str, usage: &usage::Usage) -> Vec<usize> {
    ranked(items, query, usage, false).order
}

/// Order rows for a query.
///
/// Favourites come first. While searching, the text match decides; launch
/// frecency only orders rows whose match is equally good, so a better match
/// always beats a frequently used row. With an empty query and `recent`, the
/// most recently launched rows (up to five) follow the favourites, then the
/// rest alphabetically.
pub(crate) fn ranked(items: &[Item], query: &str, usage: &usage::Usage, recent: bool) -> Ranked {
    let mut favorites: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.favorite)
        .map(|(index, _)| index)
        .collect();
    favorites.sort_by_key(|index| items[*index].label().to_lowercase());
    favorites.truncate(5);
    let pinned: std::collections::HashSet<usize> = favorites.iter().copied().collect();
    let q = query.trim().to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();
    let recent_rows: Vec<usize> = if recent && terms.is_empty() && !usage.is_empty() {
        let mut used: Vec<(u64, usize)> = items
            .iter()
            .enumerate()
            .filter(|(index, _)| !pinned.contains(index))
            .filter_map(|(index, item)| usage.last(&item.key).map(|stamp| (stamp, index)))
            .collect();
        // Newest first; one row per key even if a key appears twice.
        used.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut seen = std::collections::HashSet::new();
        used.into_iter()
            .filter(|(_, index)| seen.insert(items[*index].key.as_str()))
            .map(|(_, index)| index)
            .take(RECENT_ROWS)
            .collect()
    } else {
        Vec::new()
    };
    let recent_set: std::collections::HashSet<usize> = recent_rows.iter().copied().collect();
    let mut hits: Vec<(bool, u16, std::cmp::Reverse<u32>, usize, String, usize)> = items
        .iter()
        .enumerate()
        .filter(|(i, _)| !recent_set.contains(i))
        .filter_map(|(i, item)| {
            let label = item.label().to_lowercase();
            let rank = if terms.is_empty() {
                0
            } else {
                let fields = SearchFields::new(item);
                let mut total = 0u16;
                for term in &terms {
                    total += u16::from(fields.rank(term)?);
                }
                total
            };
            // Frecency and a shorter label are closer matches only while
            // searching; with an empty field the list stays alphabetical.
            let (frecency, length) = if terms.is_empty() {
                (0, 0)
            } else {
                (usage.score(&item.key), label.len())
            };
            Some((
                !pinned.contains(&i),
                rank,
                std::cmp::Reverse(frecency),
                length,
                label,
                i,
            ))
        })
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
            .then_with(|| a.5.cmp(&b.5))
    });
    let pinned_count = hits.iter().take_while(|hit| !hit.0).count();
    let mut order: Vec<usize> = hits.into_iter().map(|hit| hit.5).collect();
    let recent_range = pinned_count..pinned_count + recent_rows.len();
    order.splice(pinned_count..pinned_count, recent_rows);
    Ranked {
        order,
        recent: recent_range,
    }
}

pub(crate) fn system_action_priority(group: &str) -> u8 {
    match group {
        "NIRU Actions" => 0,
        "Work contexts" => 1,
        "Saved actions" => 2,
        "Diagnostics" => 3,
        "System" => 4,
        "Audio" => 5,
        "Display" => 6,
        "Layouts" => 7,
        "Appearance" => 8,
        "Settings" | "NIRUX Settings" => 9,
        "Recipes" => 10,
        "Power" => 11,
        "Shortcuts" => 12,
        "Themes" => 13,
        "Wallpaper" => 14,
        "Workspaces" => 15,
        _ => 16,
    }
}

pub(crate) fn filtered_for_mode(
    items: &[Item],
    query: &str,
    usage: &usage::Usage,
    mode: &str,
) -> Vec<usize> {
    ranked_for_mode(items, query, usage, mode).order
}

/// Views whose empty query shows recently launched rows.
pub(crate) fn shows_recent(mode: &str) -> bool {
    matches!(mode, "apps" | "actions")
}

pub(crate) fn ranked_for_mode(
    items: &[Item],
    query: &str,
    usage: &usage::Usage,
    mode: &str,
) -> Ranked {
    if mode == "agent" {
        // The query is the prompt, not a filter. Keeping the row selected
        // makes Enter predictable while the user types.
        return Ranked {
            order: (0..items.len()).collect(),
            recent: 0..0,
        };
    }
    if mode == "system-actions" && query.trim().is_empty() {
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|index| {
            (
                system_action_priority(&items[*index].group),
                items[*index].label().to_lowercase(),
            )
        });
        Ranked {
            order,
            recent: 0..0,
        }
    } else {
        ranked(items, query, usage, shows_recent(mode))
    }
}

/// Place the intent rows (appended after the plain rows) before every ordinary
/// search result.
pub(crate) fn intent_first_order(
    base_len: usize,
    intent_count: usize,
    base_order: Vec<usize>,
) -> Vec<usize> {
    if intent_count == 0 {
        return base_order;
    }
    (base_len..base_len + intent_count)
        .chain(base_order)
        .collect()
}

/// Mark configured and runtime favourites, then keep the first five by label.
pub(crate) fn mark_favorites(items: &mut [Item], cfg: &Config) {
    let runtime_favorites = load_runtime_favorites();
    let removed_favorites = load_removed_favorites();
    for item in items.iter_mut() {
        let configured = cfg.favorites.iter().any(|favorite| {
            favorite == &item.key
                || favorite.eq_ignore_ascii_case(&item.name)
                || favorite == &item.label()
        });
        item.favorite =
            (item.favorite || configured || runtime_favorites.iter().any(|key| key == &item.key))
                && !removed_favorites.iter().any(|key| key == &item.key);
    }
    let mut favorite_items: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.favorite)
        .map(|(index, _)| index)
        .collect();
    favorite_items.sort_by_key(|index| items[*index].label().to_lowercase());
    for index in favorite_items.into_iter().skip(5) {
        items[index].favorite = false;
    }
}

#[cfg(test)]
mod favourite_tests {

    fn searchable(key: &str, group: &str, name: &str) -> crate::Item {
        crate::Item {
            key: key.into(),
            group: group.into(),
            name: name.into(),
            detail: String::new(),
            keywords: String::new(),
            favorite: false,
            target: crate::Target::Confirm(crate::Action::Logout),
        }
    }

    #[test]
    fn favourites_and_local_recency_order_the_results() {
        let mut items = vec![
            searchable("a", "Applications", "Alpha"),
            searchable("b", "Applications", "Beta"),
            searchable("c", "Applications", "Gamma"),
        ];
        items[1].favorite = true;
        let usage = crate::usage::Usage::from_rows([("c", 20), ("a", 10)], 30);
        let order = crate::filtered(&items, "", &usage);
        assert_eq!(order, vec![1, 0, 2], "favourites first, then alphabetical");
    }

    #[test]
    fn group_names_are_searchable() {
        let items = vec![searchable("w1", "Windows", "Editor")];
        let usage = crate::usage::Usage::default();
        assert_eq!(crate::filtered(&items, "windows", &usage), vec![0]);
        assert_eq!(crate::filtered(&items, "editor", &usage), vec![0]);
        assert!(crate::filtered(&items, "missing", &usage).is_empty());
    }
}

#[cfg(test)]
mod search_tests {
    use crate::{filtered, Item, Target};

    const DAY: u64 = 86_400;

    fn row(group: &str, name: &str, detail: &str) -> Item {
        Item {
            key: format!("{group}:{name}"),
            group: group.into(),
            name: name.into(),
            detail: detail.into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }
    }

    fn names(items: &[Item], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| items[i].name.clone()).collect()
    }

    #[test]
    fn the_matched_part_of_a_row_is_highlighted() {
        let item = row("Applications", "Firefox", "Web browser");
        // "Applications · Firefox": the name starts at column 15.
        let found = crate::highlight(&item, "fire");
        assert_eq!(found.positions, vec![15, 16, 17, 18]);
        assert_eq!(found.keyword, None);
        // A word start wins over an earlier inner match.
        let item = row("", "Spotify Spot", "");
        assert_eq!(crate::highlight(&item, "spot").positions, vec![0, 1, 2, 3]);
        // A subsequence shows each matched letter.
        let item = row("", "Thunderbird", "");
        assert_eq!(crate::highlight(&item, "tbd").positions, vec![0, 7, 10]);
        // Folded: e marks É.
        let item = row("", "Écran", "");
        assert_eq!(crate::highlight(&item, "ecr").positions, vec![0, 1, 2]);
        // Only a keyword matched: name the keyword, underline nothing.
        let mut item = row("Applications", "Kitty", "kitty.desktop");
        item.keywords = "terminal shell tty".into();
        let found = crate::highlight(&item, "shell");
        assert!(found.positions.is_empty());
        assert_eq!(found.keyword.as_deref(), Some("shell"));
        let item = row("Audio", "Volume +5%", "Raise the default output volume");
        assert_eq!(
            crate::highlight(&item, "raise").keyword.as_deref(),
            Some("Raise")
        );
    }

    #[test]
    fn matching_ignores_diacritics_but_ranks_exact_text_first() {
        let items = vec![
            row("Applications", "Écran", "screen.desktop"),
            row("Applications", "Ecrin", "case.desktop"),
            row("Applications", "Skärmläsare", "orca.desktop"),
            row("Applications", "Café", "cafe.desktop"),
        ];
        let usage = crate::usage::Usage::default();
        // A plain e finds É and é.
        assert_eq!(
            names(&items, &filtered(&items, "ecr", &usage)),
            ["Ecrin", "Écran"]
        );
        assert_eq!(names(&items, &filtered(&items, "cafe", &usage)), ["Café"]);
        // a matches ä, and the Swedish spelling still matches exactly.
        assert_eq!(
            names(&items, &filtered(&items, "skarm", &usage)),
            ["Skärmläsare"]
        );
        assert_eq!(
            names(&items, &filtered(&items, "skärm", &usage)),
            ["Skärmläsare"]
        );
        // é typed with a dead key ranks the exact É row first.
        assert_eq!(
            names(&items, &filtered(&items, "écr", &usage)),
            ["Écran", "Ecrin"]
        );
    }

    #[test]
    fn frecency_orders_equal_matches_while_searching() {
        let items = vec![
            row("Applications", "Files", "files.desktop"),
            row("Applications", "Firefox", "firefox.desktop"),
            row("Applications", "Fish", "fish.desktop"),
        ];
        let now = 50 * DAY;
        let usage = crate::usage::Usage::from_rows(
            [
                ("Applications:Firefox", now - DAY),
                ("Applications:Firefox", now - 2 * DAY),
                ("Applications:Fish", now - 40 * DAY),
            ],
            now,
        );
        // All three are prefix matches: daily Firefox, then the old Fish, then
        // the never used Files.
        let order = filtered(&items, "fi", &usage);
        assert_eq!(names(&items, &order), ["Firefox", "Fish", "Files"]);
        // Without history the shorter name wins the tie, as before.
        let order = filtered(&items, "fi", &crate::usage::Usage::default());
        assert_eq!(names(&items, &order), ["Fish", "Files", "Firefox"]);
    }

    #[test]
    fn a_better_text_match_beats_more_frequent_use() {
        let items = vec![
            row("Applications", "Terminal", "kitty.desktop"),
            row("Applications", "Kitty", "kitty.desktop"),
            row("Applications", "Kit builder", "kit.desktop"),
        ];
        let now = 50 * DAY;
        let rows: Vec<(&str, u64)> = (0..30)
            .map(|day| ("Applications:Kit builder", now - day * 3600))
            .collect();
        let usage = crate::usage::Usage::from_rows(rows, now);
        // "kitty" is an exact name for Kitty; the heavily used row only has a
        // prefix-free partial match and must stay behind it.
        let order = filtered(&items, "kitty", &usage);
        assert_eq!(names(&items, &order)[0], "Kitty");
        // An exact name beats a frequently used prefix match.
        let items = vec![
            row("Applications", "Code", "code.desktop"),
            row("Applications", "Codeblocks", "cb.desktop"),
        ];
        let rows: Vec<(&str, u64)> = (0..30)
            .map(|hour| ("Applications:Codeblocks", now - hour * 3600))
            .collect();
        let usage = crate::usage::Usage::from_rows(rows, now);
        assert_eq!(names(&items, &filtered(&items, "code", &usage))[0], "Code");
    }

    #[test]
    fn an_empty_query_shows_recent_rows_after_the_favourites() {
        let mut items: Vec<Item> = [
            "Alpha", "Beta", "Gamma", "Delta", "Echo", "Foxtrot", "Golf", "Hotel",
        ]
        .iter()
        .map(|name| row("Applications", name, ""))
        .collect();
        items[7].favorite = true; // Hotel
        let now = 50 * DAY;
        let rows: Vec<(&str, u64)> = [
            ("Applications:Golf", 10),
            ("Applications:Foxtrot", 9),
            ("Applications:Echo", 8),
            ("Applications:Delta", 7),
            ("Applications:Gamma", 6),
            ("Applications:Beta", 5),
            ("Applications:Hotel", 11),
        ]
        .into_iter()
        .map(|(key, offset)| (key, now - DAY + offset))
        .collect();
        let usage = crate::usage::Usage::from_rows(rows, now);
        let ranked = crate::ranked(&items, "", &usage, true);
        assert_eq!(
            names(&items, &ranked.order),
            ["Hotel", "Golf", "Foxtrot", "Echo", "Delta", "Gamma", "Alpha", "Beta"],
            "favourite, five recent (newest first), then the rest alphabetically"
        );
        assert_eq!(ranked.recent, 1..6);
        // Searching drops the recent block; views without it stay alphabetical.
        assert!(crate::ranked(&items, "a", &usage, true).recent.is_empty());
        assert!(crate::ranked(&items, "", &usage, false).recent.is_empty());
        assert!(crate::shows_recent("apps") && crate::shows_recent("actions"));
        assert!(!crate::shows_recent("sound") && !crate::shows_recent("settings"));
    }

    /// A row is found by its group name, because the group is part of what the
    /// list calls it. Several groups rely on this instead of repeating the
    /// word in every row's name.
    #[test]
    fn an_empty_query_stays_alphabetical_not_shortest_first() {
        let items = vec![
            row("Applications", "Zeta", "z.desktop"),
            row("Applications", "Alpha", "a.desktop"),
        ];
        let usage = crate::usage::Usage::default();
        let order = filtered(&items, "", &usage);
        let names: Vec<&str> = order.iter().map(|&i| items[i].name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Zeta"]);
    }

    #[test]
    fn a_row_is_found_by_its_group_name() {
        let items = vec![
            row("Appearance", "Compositor", "Toggle transparency"),
            row("Files", "Backup", "Copy to a disk"),
        ];
        let usage = crate::usage::Usage::default();
        let hits: Vec<&str> = filtered(&items, "appearance", &usage)
            .into_iter()
            .map(|i| items[i].name.as_str())
            .collect();
        assert_eq!(hits, ["Compositor"]);
    }

    /// Within one rank a shorter label is the closer match and sorts first.
    #[test]
    fn a_closer_match_sorts_before_a_longer_one() {
        let items = vec![
            row("Wallpaper", "Wallpaper wallpaper wallpaper", "x"),
            row("Wallpaper", "Wallpaper", "y"),
        ];
        let usage = crate::usage::Usage::default();
        let order = filtered(&items, "wallpaper", &usage);
        assert_eq!(
            items[order[0]].name, "Wallpaper",
            "the shorter label is the closer match"
        );
    }

    #[test]
    fn multiword_keyword_and_fuzzy_matching() {
        let mut firefox = row("Applications", "Firefox", "firefox.desktop");
        firefox.keywords = "web browser internet".into();
        let items = vec![firefox, row("Applications", "Files", "files.desktop")];
        let usage = crate::usage::Usage::default();
        assert_eq!(filtered(&items, "web browser", &usage), vec![0]);
        assert_eq!(filtered(&items, "firfox", &usage), vec![0]);
        assert!(filtered(&items, "web files", &usage).is_empty());
    }
}
