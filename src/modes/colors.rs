//! The Colors view (`--colors`): the last colors NIRUSHOT picked, with a
//! swatch, and a row that picks a new one.
//!
//! `nirushot pick` keeps `~/.local/state/nirushot/colors.json`, a JSON array,
//! newest first, of `{"hex": "#rrggbb", "time": unix seconds}`.

use crate::*;

pub(crate) const KEY_PREFIX: &str = "color:";
const SHOWN: usize = 10;

fn history_path() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/state")
        })
        .join("nirushot/colors.json")
}

#[derive(serde::Deserialize)]
struct Pick {
    hex: String,
    #[serde(default)]
    time: u64,
}

/// `#rrggbb` as 0xrrggbb, for a valid six-digit hex color.
pub(crate) fn parse_hex(hex: &str) -> Option<u32> {
    let h = hex.strip_prefix('#')?;
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
}

/// The color a row's swatch shows.
pub(crate) fn swatch(key: &str) -> Option<u32> {
    let rest = key.strip_prefix(KEY_PREFIX)?;
    parse_hex(rest.split(':').next()?)
}

fn ago(time: u64, now: u64) -> String {
    if time == 0 || time > now {
        return String::new();
    }
    let s = now - time;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

fn rgb_text(c: u32) -> String {
    format!("rgb({}, {}, {})", c >> 16, (c >> 8) & 255, c & 255)
}

pub(crate) fn pick_item() -> Item {
    Item {
        key: "colors:pick".into(),
        group: "Colors".into(),
        name: "Pick color".into(),
        detail: if in_path("nirushot") {
            "Super+Ctrl+C · nirushot pick".into()
        } else {
            "needs NIRUSHOT".into()
        },
        keywords: "pick color colour picker eyedropper pipett färg nirushot".into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["nirushot".into(), "pick".into()],
            terminal: false,
            path: None,
        },
    }
}

pub(crate) fn color_rows(text: &str, now: u64) -> Vec<Item> {
    let picks: Vec<Pick> = serde_json::from_str(text).unwrap_or_default();
    let mut seen: Vec<String> = Vec::new();
    let mut items = Vec::new();
    for pick in picks {
        let hex = pick.hex.trim().to_lowercase();
        let Some(color) = parse_hex(&hex) else {
            continue;
        };
        if seen.contains(&hex) {
            continue;
        }
        seen.push(hex.clone());
        let when = ago(pick.time, now);
        items.push(Item {
            key: format!("{KEY_PREFIX}{hex}:{}", pick.time),
            group: "Colors".into(),
            name: hex.clone(),
            detail: if when.is_empty() {
                rgb_text(color)
            } else {
                format!("{} · {when}", rgb_text(color))
            },
            keywords: format!("color colour färg {}", rgb_text(color)),
            favorite: false,
            target: Target::Copy(hex),
        });
        if items.len() == SHOWN {
            break;
        }
    }
    items
}

/// Pick first, then the history (or a line saying there is none yet).
pub(crate) fn colors_items() -> Vec<Item> {
    let mut items = vec![pick_item()];
    let text = std::fs::read_to_string(history_path()).unwrap_or_default();
    let rows = color_rows(&text, usage::now());
    if rows.is_empty() {
        items.push(Item {
            key: "colors:none".into(),
            group: "Colors".into(),
            name: "No colors picked yet".into(),
            detail: "Super+Ctrl+C picks one".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    items.extend(rows);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rows_have_swatches_and_copy_the_hex() {
        let json = r##"[{"hex":"#FF8800","time":990},{"hex":"#ff8800","time":900},
            {"hex":"bad","time":1},{"hex":"#0a0b0c","time":0}]"##;
        let rows = color_rows(json, 1000);
        assert_eq!(rows.len(), 2, "duplicates and invalid entries are skipped");
        assert_eq!(rows[0].name, "#ff8800");
        assert_eq!(swatch(&rows[0].key), Some(0xff8800));
        assert!(rows[0].detail.starts_with("rgb(255, 136, 0)"));
        assert!(rows[0].detail.ends_with("just now"));
        assert!(matches!(&rows[0].target, Target::Copy(t) if t == "#ff8800"));
        assert_eq!(rows[1].detail, "rgb(10, 11, 12)");
        assert!(color_rows("not json", 0).is_empty());
    }

    #[test]
    fn at_most_ten_colors_are_shown() {
        let many: Vec<String> = (0..15)
            .map(|i| format!("{{\"hex\":\"#0000{i:02x}\",\"time\":{i}}}"))
            .collect();
        let rows = color_rows(&format!("[{}]", many.join(",")), 100);
        assert_eq!(rows.len(), 10);
    }
}
