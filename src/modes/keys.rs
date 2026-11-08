//! The keybinding cheat sheet (`--keys`, Super+/).
//!
//! Its rows come from `niructl keys --json`: NIRUWM's live table with an
//! English description for every action, plus the keys other programs
//! register in `~/.local/share/niru/keys.d`. Enter runs a binding through
//! `niructl keys run`, exactly as if the key were pressed; Ctrl+R tells what
//! a pressed combination does. An older niructl without `--json` still gets
//! the plain list.

use crate::*;

pub(crate) const KEY_PREFIX: &str = "keys:";
const HEADING: &str = "keyshead:";

pub(crate) fn is_heading(item: &Item) -> bool {
    item.key.starts_with(HEADING)
}
/// Keywords carry the key as `#k<mods>-<keysym>`, so a typed or pressed
/// combination finds its row exactly.
const TOKEN: &str = "#k";
/// Conflict text after this separator in the keywords; the footer shows it.
pub(crate) const NOTE: char = '\u{1f}';

/// The X modifier bits a binding uses: Shift, Control, Mod1 (Alt), Mod3,
/// Mod4 (Super). Lock and NumLock never count.
pub(crate) const MODS: u32 = 0x1 | 0x4 | 0x8 | 0x20 | 0x40;

#[derive(serde::Deserialize)]
struct Sheet {
    #[serde(default)]
    entries: Vec<Entry>,
    #[serde(default)]
    problems: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Entry {
    keys: String,
    label: String,
    #[serde(default)]
    mods: u32,
    #[serde(default)]
    keysym: u32,
    action: Option<String>,
    run: Option<Vec<String>>,
    description: String,
    group: String,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    conflicts: Vec<String>,
}

/// The order NIRUWM lists its groups in; registered apps follow.
const GROUPS: &[&str] = &[
    "Apps and launchers",
    "Session",
    "Layouts",
    "Scratchpads",
    "Focus and windows",
    "Workspaces",
    "Floating windows",
    "Screenshots and color",
    "Hardware keys",
];

pub(crate) fn token(mods: u32, keysym: u32) -> String {
    format!("{TOKEN}{}-{keysym:x}", mods & MODS)
}

fn rows(sheet: Sheet) -> Vec<Item> {
    let mut entries = sheet.entries;
    // Stable: NIRUWM's table order inside each group.
    entries.sort_by_key(|e| {
        GROUPS
            .iter()
            .position(|g| *g == e.group)
            .unwrap_or(GROUPS.len())
    });
    let items: Vec<Item> = entries
        .into_iter()
        .map(|e| {
            let who = e.app.clone().unwrap_or_else(|| "NIRUWM".into());
            let target = match (&e.action, &e.run) {
                (Some(_), _) => Target::RunKey(e.keys.clone()),
                (None, Some(argv)) => Target::Spawn {
                    argv: argv.clone(),
                    terminal: false,
                    path: None,
                },
                (None, None) => Target::Info,
            };
            let conflict = !e.conflicts.is_empty();
            let mut keywords = format!(
                "{} {} {} {} {}",
                e.keys,
                e.label,
                e.action.as_deref().unwrap_or(""),
                who,
                token(e.mods, e.keysym)
            );
            if conflict {
                keywords.push_str(" conflict ");
                keywords.push(NOTE);
                keywords.push_str(&format!(
                    "{} is also used by {}",
                    e.label,
                    e.conflicts.join("; ")
                ));
            }
            Item {
                key: format!("{KEY_PREFIX}{}:{}", who, e.keys),
                group: e.group,
                name: e.description,
                detail: if conflict {
                    format!("{} · conflict", e.label)
                } else {
                    e.label
                },
                keywords,
                favorite: false,
                target,
            }
        })
        .collect();
    // A heading row before each group; searching hides them.
    let mut grouped: Vec<Item> = Vec::with_capacity(items.len() + 16);
    for item in items {
        if grouped
            .iter()
            .rev()
            .find(|i| !is_heading(i))
            .map_or(true, |prev| prev.group != item.group)
        {
            grouped.push(Item {
                key: format!("{HEADING}{}", item.group),
                group: String::new(),
                name: item.group.clone(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
        grouped.push(item);
    }
    let mut items = grouped;
    for problem in sheet.problems {
        items.push(Item {
            key: format!("{KEY_PREFIX}problem:{problem}"),
            group: "Problems".into(),
            name: "A keys.d file could not be read".into(),
            detail: problem,
            keywords: "problem error keys.d".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    items
}

pub(crate) fn parse_sheet(text: &str) -> Option<Vec<Item>> {
    let sheet: Sheet = serde_json::from_str(text).ok()?;
    (!sheet.entries.is_empty()).then(|| rows(sheet))
}

/// The cheat sheet's rows, or the older plain list when niructl has no
/// `--json` yet.
pub(crate) fn keys_items() -> Vec<Item> {
    if let Some(out) = bounded_output(
        "niructl",
        &["keys", "--json"],
        std::time::Duration::from_millis(800),
    ) {
        if out.status.success() {
            if let Some(items) = parse_sheet(&String::from_utf8_lossy(&out.stdout)) {
                return items;
            }
        }
    }
    shortcut_items()
}

/// A typed combination (`super+t`, `Super T`, `ctrl alt space`) as
/// modifiers and the keysym NIRUWM would bind; `None` for ordinary words.
pub(crate) fn typed_combo(query: &str) -> Option<(u32, u32)> {
    let parts: Vec<String> = query
        .trim()
        .split(|c: char| c == '+' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(str::to_lowercase)
        .collect();
    if parts.len() < 2 && !query.contains('+') {
        return None;
    }
    let (key, mods) = parts.split_last()?;
    let mut mask = 0;
    for m in mods {
        mask |= match m.as_str() {
            "super" | "win" | "mod4" | "meta" => 0x40,
            "ctrl" | "control" | "ctl" => 0x4,
            "alt" | "mod1" => 0x8,
            "shift" => 0x1,
            _ => return None,
        };
    }
    let sym = match key.as_str() {
        "enter" | "return" => 0xff0d,
        "space" => 0x20,
        "tab" => 0xff09,
        "esc" | "escape" => 0xff1b,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "print" => 0xff61,
        "comma" => 0x2c,
        "period" | "dot" => 0x2e,
        "slash" => 0x2f,
        "grave" | "backtick" => 0x60,
        "minus" => 0x2d,
        "equal" => 0x3d,
        k if k.chars().count() == 1 => {
            let c = k.chars().next()?;
            u32::from(c.to_ascii_lowercase())
        }
        k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=35).contains(&n)) => {
            0xffbe + k[1..].parse::<u32>().ok()? - 1
        }
        _ => return None,
    };
    Some((mask, sym))
}

/// Rows bound to exactly `token` first, then `rest` without them.
pub(crate) fn exact_first(items: &[Item], token: &str, rest: Vec<usize>) -> Vec<usize> {
    let has = |i: &usize| {
        items[*i]
            .keywords
            .split(NOTE)
            .next()
            .unwrap_or("")
            .split_whitespace()
            .any(|w| w == token)
    };
    let mut exact: Vec<usize> = (0..items.len()).filter(has).collect();
    exact.extend(rest.into_iter().filter(|i| !has(i)));
    exact
}

/// The conflict note of a row, for the footer.
pub(crate) fn note(item: &Item) -> Option<&str> {
    item.keywords.split_once(NOTE).map(|(_, n)| n)
}

/// The keysym NIRUWM would see for a pressed key: the first symbol of its
/// keycode (letters lower case), so Super+Shift+1 is `1` with Shift.
pub(crate) fn pressed_keysym(keymap: &Keymap, code: u8) -> u32 {
    let sym = keymap.keysyms(code).first().copied().unwrap_or(0);
    if (0x41..=0x5a).contains(&sym) {
        sym + 0x20
    } else {
        sym
    }
}

/// True for a key that is only a modifier (Shift, Control, Super …).
pub(crate) fn is_modifier(sym: u32) -> bool {
    (0xffe1..=0xffee).contains(&sym) || sym == 0xfe03 || sym == 0xff7e
}

/// A pressed combination as people write it.
pub(crate) fn combo_label(mods: u32, sym: u32) -> String {
    let mut out = String::new();
    for (bit, name) in [
        (0x40, "Super"),
        (0x4, "Ctrl"),
        (0x8, "Alt"),
        (0x20, "Mod3"),
        (0x1, "Shift"),
    ] {
        if mods & bit != 0 {
            out.push_str(name);
            out.push('+');
        }
    }
    let key = match sym {
        0xff0d => "Enter".to_string(),
        0x20 => "Space".to_string(),
        0xff09 => "Tab".to_string(),
        0xff1b => "Escape".to_string(),
        0xff51 => "Left".to_string(),
        0xff52 => "Up".to_string(),
        0xff53 => "Right".to_string(),
        0xff54 => "Down".to_string(),
        0xff61 => "Print".to_string(),
        0xff08 => "BackSpace".to_string(),
        0xffbe..=0xffe0 => format!("F{}", sym - 0xffbe + 1),
        s => match char::from_u32(s) {
            Some(c) if !c.is_control() => c.to_uppercase().collect(),
            _ => format!("keysym 0x{s:x}"),
        },
    };
    out.push_str(&key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"version":1,"entries":[
      {"keys":"super+t","label":"Super+T","mods":64,"keysym":116,"action":"layout tile","run":null,
       "description":"Layout: Tile","group":"Layouts","source":"niruwm","app":null,
       "conflicts":["NIRUDICTATE: Clash"]},
      {"keys":"super+Return","label":"Super+Enter","mods":64,"keysym":65293,"action":"terminal","run":null,
       "description":"Open a terminal","group":"Apps and launchers","source":"niruwm","app":null,"conflicts":[]},
      {"keys":"ctrl+alt+space","label":"Ctrl+Alt+Space","mods":12,"keysym":32,"action":null,
       "run":["nirudictate","toggle"],"description":"Dictate","group":"Dictation","source":"app",
       "app":"NIRUDICTATE","conflicts":[]}],
      "problems":["/x/broken.toml: invalid"]}"#;

    #[test]
    fn the_sheet_is_grouped_and_every_row_can_act() {
        let all = parse_sheet(JSON).unwrap();
        let heads: Vec<&str> = all
            .iter()
            .filter(|i| is_heading(i))
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(heads, ["Apps and launchers", "Layouts", "Dictation"]);
        let items: Vec<&Item> = all.iter().filter(|i| !is_heading(i)).collect();
        assert_eq!(items[0].name, "Open a terminal", "groups in NIRUWM order");
        assert_eq!(items[1].name, "Layout: Tile");
        assert_eq!(items[1].detail, "Super+T · conflict");
        assert_eq!(
            note(items[1]),
            Some("Super+T is also used by NIRUDICTATE: Clash")
        );
        assert!(matches!(&items[1].target, Target::RunKey(k) if k == "super+t"));
        assert!(matches!(&items[2].target, Target::Spawn { argv, .. } if argv[0] == "nirudictate"));
        assert_eq!(items[3].group, "Problems");
        assert!(parse_sheet("{}").is_none());
    }

    #[test]
    fn typed_and_pressed_combinations_find_their_row() {
        assert_eq!(typed_combo("super+t"), Some((0x40, 0x74)));
        assert_eq!(typed_combo("Super T"), Some((0x40, 0x74)));
        assert_eq!(typed_combo("ctrl alt space"), Some((0xc, 0x20)));
        assert_eq!(typed_combo("super+shift+Return"), Some((0x41, 0xff0d)));
        assert_eq!(typed_combo("super+/"), Some((0x40, 0x2f)));
        assert_eq!(typed_combo("layout tile"), None);
        assert_eq!(typed_combo("screen"), None);
        let items = parse_sheet(JSON).unwrap();
        let t = token(0x40, 0x74);
        let order = exact_first(&items, &t, (0..items.len()).collect());
        assert_eq!(items[order[0]].name, "Layout: Tile");
        assert_eq!(order.len(), items.len());
        assert_eq!(combo_label(0x41, 0x31), "Super+Shift+1");
        assert_eq!(combo_label(0x40, 0x2f), "Super+/");
        assert_eq!(combo_label(0x4, 0xffbe + 4), "Ctrl+F5");
    }
}
