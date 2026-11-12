//! The Manual view (`--manual`, NIRU Actions "manual", "help", "hjälp").
//!
//! It reads the manual of the installed NIRUX release from the directory
//! `nirux manual --path` prints: one row per chapter and per `##`/`###`
//! heading, searchable by heading and by the text under it. Enter runs
//! `nirux manual CHAPTER#ANCHOR`, which opens the chapter at that heading.
//! The anchors follow the same rule as `nirux manual` (and GitHub): ASCII
//! lowercased, punctuation dropped, spaces to hyphens, `-1`, `-2` ... for a
//! repeated heading in one file.

use crate::*;

pub(crate) const MANUAL_PREFIX: &str = "manual:";
const CHAPTER: &str = "manual:chapter:";
/// Text under a heading that is searched, in characters.
const TEXT_BUDGET: usize = 1200;

pub(crate) fn is_chapter(item: &Item) -> bool {
    item.key.starts_with(CHAPTER)
}

/// One heading of a chapter file.
#[derive(Debug, PartialEq)]
pub(crate) struct Heading {
    pub(crate) level: usize,
    pub(crate) text: String,
    pub(crate) anchor: String,
    /// Plain text from this heading to the next one.
    pub(crate) body: String,
}

/// GitHub's (and `nirux manual`'s) anchor of a heading.
pub(crate) fn anchor(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            let ch = ch.to_ascii_lowercase();
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                out.push(ch);
            } else if ch == ' ' {
                out.push('-');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn plain(line: &str) -> String {
    line.chars()
        .filter(|c| !matches!(c, '`' | '*' | '|' | '#' | '<' | '>'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The headings of one Markdown file, outside code blocks. Level 1 is the
/// chapter title.
pub(crate) fn headings(text: &str) -> Vec<Heading> {
    let mut out: Vec<Heading> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut fence = false;
    for line in text.lines() {
        if line.starts_with("```") || line.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if !fence && level > 0 && line[level..].starts_with(' ') {
            let mut title = line[level + 1..].trim_end();
            title = title.trim_end_matches('#').trim_end();
            let mut slug = anchor(&title.replace('`', ""));
            match seen.get_mut(&slug) {
                Some(count) => {
                    *count += 1;
                    slug = format!("{slug}-{count}");
                }
                None => {
                    seen.insert(slug.clone(), 0);
                }
            }
            out.push(Heading {
                level,
                text: title.replace('`', ""),
                anchor: slug,
                body: String::new(),
            });
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.body.chars().count() < TEXT_BUDGET {
                let words = plain(line);
                if !words.is_empty() {
                    if !last.body.is_empty() {
                        last.body.push(' ');
                    }
                    last.body.push_str(&words);
                }
            }
        }
    }
    for heading in &mut out {
        if heading.body.chars().count() > TEXT_BUDGET {
            heading.body = heading.body.chars().take(TEXT_BUDGET).collect();
        }
    }
    out
}

/// The chapter files of a manual directory, in order: the index first.
pub(crate) fn chapter_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    name.ends_with(".md")
                        && name.len() > 3
                        && name.as_bytes()[0].is_ascii_digit()
                        && name.as_bytes()[1].is_ascii_digit()
                        && name.as_bytes()[2] == b'-'
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let index = dir.join("README.md");
    if index.is_file() {
        files.insert(0, index);
    }
    files
}

fn open(topic: String) -> Target {
    Target::Spawn {
        argv: vec!["nirux".into(), "manual".into(), topic],
        terminal: false,
        path: None,
    }
}

/// The rows of a manual directory.
pub(crate) fn rows(dir: &std::path::Path) -> Vec<Item> {
    let mut items = Vec::new();
    for file in chapter_files(dir) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let stem = name.trim_end_matches(".md").to_string();
        let index = name == "README.md";
        let all = headings(&text);
        let title = all
            .iter()
            .find(|h| h.level == 1)
            .map(|h| h.text.clone())
            .unwrap_or_else(|| stem.clone());
        let number = if index {
            String::new()
        } else {
            stem.split('-')
                .next()
                .unwrap_or("")
                .trim_start_matches('0')
                .to_string()
        };
        let intro = all
            .iter()
            .find(|h| h.level == 1)
            .map(|h| h.body.clone())
            .unwrap_or_default();
        items.push(Item {
            key: format!("{CHAPTER}{stem}"),
            group: String::new(),
            name: if index {
                title.clone()
            } else {
                format!("{number}. {title}")
            },
            detail: if index {
                "Contents".into()
            } else {
                "Chapter".into()
            },
            keywords: format!("manual chapter {stem} {intro}"),
            favorite: false,
            target: open(if index {
                "README.md".into()
            } else {
                stem.clone()
            }),
        });
        for heading in all
            .iter()
            .filter(|h| (h.level == 2 || h.level == 3) && h.text != "See also")
        {
            items.push(Item {
                key: format!("{MANUAL_PREFIX}{stem}#{}", heading.anchor),
                group: String::new(),
                name: if heading.level == 3 {
                    format!("    {}", heading.text)
                } else {
                    format!("  {}", heading.text)
                },
                detail: title.clone(),
                keywords: heading.body.clone(),
                favorite: false,
                target: open(format!("{stem}#{}", heading.anchor)),
            });
        }
    }
    items
}

fn unavailable(detail: &str) -> Vec<Item> {
    vec![Item {
        key: format!("{MANUAL_PREFIX}missing"),
        group: String::new(),
        name: "The NIRUX manual is not installed".into(),
        detail: detail.into(),
        keywords: "manual help".into(),
        favorite: false,
        target: Target::Info,
    }]
}

/// The Manual view's rows, from the manual `nirux manual --path` names.
pub(crate) fn manual_items() -> Vec<Item> {
    let Some(out) = bounded_output(
        "nirux",
        &["manual", "--path"],
        std::time::Duration::from_millis(1500),
    ) else {
        return unavailable("nirux was not found");
    };
    if !out.status.success() {
        return unavailable("It ships with NIRUX releases after 0.12.245 · nirux update");
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let dir = std::path::PathBuf::from(path);
    let items = rows(&dir);
    if items.is_empty() {
        unavailable("The manual directory has no chapters · nirux update")
    } else {
        items
    }
}

/// The NIRU Actions row that opens this view.
pub(crate) fn action_item() -> Item {
    Item {
        key: "action:manual".into(),
        group: "NIRU Actions".into(),
        name: "Manual".into(),
        detail: "Read the NIRUX manual · chapters and headings".into(),
        keywords: "manual help hjälp hjalp handbok documentation docs guide howto".into(),
        favorite: false,
        target: Target::Mode("manual".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_follow_github() {
        assert_eq!(
            anchor("Lock screen and screensaver"),
            "lock-screen-and-screensaver"
        );
        assert_eq!(
            anchor("NIRUDICTATE: local dictation"),
            "nirudictate-local-dictation"
        );
        assert_eq!(
            anchor("~/.config/niruwm/config.toml"),
            "configniruwmconfigtoml"
        );
        assert_eq!(anchor("Use clipboard history"), "use-clipboard-history");
        assert_eq!(anchor("Hávamál quotes"), "hávamál-quotes");
        assert_eq!(anchor("Scroll (one or two)"), "scroll-one-or-two");
    }

    #[test]
    fn headings_skip_code_and_number_repeats() {
        let text = "# Title\nIntro text.\n\n## One\nBody `code` here.\n```sh\n# not a heading\n```\n## One\n### Sub `x`\nMore.\n";
        let found = headings(text);
        let anchors: Vec<&str> = found.iter().map(|h| h.anchor.as_str()).collect();
        assert_eq!(anchors, ["title", "one", "one-1", "sub-x"]);
        assert_eq!(found[0].body, "Intro text.");
        assert!(found[1].body.contains("Body code here."));
        assert!(found[1].body.contains("not a heading"));
        assert_eq!(found[3].text, "Sub x");
        assert_eq!(found[3].level, 3);
    }

    fn manual_dir() -> std::path::PathBuf {
        // Tests run in parallel in one process: each needs its own folder.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nirulauncher-manual-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "# The NIRUX manual\n\n## Chapters\n").unwrap();
        std::fs::write(
            dir.join("06-lock-and-screensaver.md"),
            "# Lock screen and screensaver\n\nLocking.\n\n## SID pause music\n\nCommodore 64 tunes.\n\n## See also\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("01-getting-started.md"),
            "# Getting started\n\n## First login\n\nPress Super+Enter.\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        dir
    }

    #[test]
    fn rows_list_chapters_and_headings_in_order() {
        let dir = manual_dir();
        let items = rows(&dir);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "The NIRUX manual",
                "  Chapters",
                "1. Getting started",
                "  First login",
                "6. Lock screen and screensaver",
                "  SID pause music",
            ]
        );
        assert!(is_chapter(&items[2]));
        assert!(!is_chapter(&items[3]));
        let Target::Spawn { argv, .. } = &items[5].target else {
            panic!("a heading opens the manual");
        };
        assert_eq!(
            argv,
            &["nirux", "manual", "06-lock-and-screensaver#sid-pause-music"]
        );
        let Target::Spawn { argv, .. } = &items[4].target else {
            panic!("a chapter opens the manual");
        };
        assert_eq!(argv, &["nirux", "manual", "06-lock-and-screensaver"]);
        let Target::Spawn { argv, .. } = &items[0].target else {
            panic!("the index opens the manual");
        };
        assert_eq!(argv, &["nirux", "manual", "README.md"]);
        assert!(items[5].keywords.contains("Commodore"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_finds_text_under_a_heading() {
        let dir = manual_dir();
        let items = rows(&dir);
        let usage = usage::Usage::default();
        let order = ranked_for_mode(&items, "commodore", &usage, "manual").order;
        assert_eq!(items[order[0]].name, "  SID pause music");
        let all = ranked_for_mode(&items, "", &usage, "manual").order;
        assert_eq!(all, (0..items.len()).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_manual_is_one_info_row() {
        let rows = unavailable("x");
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].target, Target::Info));
    }
}
