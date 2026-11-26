//! The panel's search: one field finds apps (the tiles at the top) and,
//! in groups below them, key bindings, settings and actions, computers,
//! manual topics and files. Every group reuses the launcher's own rows and
//! ranking: the settings and actions are the Actions view's static rows,
//! the key bindings `niructl keys --json`, the manual the Manual view's
//! rows and the files the Files view's search (`modes::files`). Nothing is
//! read until the first letter is typed; each source is read once, on a
//! worker, and kept while the panel is open.
use crate::tailscale::Device;
use crate::*;

/// The groups the search can show, in the order they are drawn.
pub(crate) const SEARCH: [&str; 6] = ["apps", "keys", "settings", "computers", "manual", "files"];

/// A result group below the apps (the files have a column of their own).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Group {
    Settings,
    Keys,
    Computers,
    Manual,
}

impl Group {
    /// The groups, in the order they are drawn.
    /// Key bindings first: what a binding does is the words people type
    /// ("close window"), and Enter runs it as it did before.
    pub(crate) const NEAR: [Group; 4] = [
        Group::Keys,
        Group::Settings,
        Group::Computers,
        Group::Manual,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Group::Settings => "settings",
            Group::Keys => "keys",
            Group::Computers => "computers",
            Group::Manual => "manual",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Group::Settings => "SETTINGS AND ACTIONS",
            Group::Keys => "KEY BINDINGS",
            Group::Computers => "COMPUTERS",
            Group::Manual => "MANUAL",
        }
    }

    /// The most rows one search keeps.
    pub(crate) fn cap(self) -> usize {
        match self {
            Group::Keys => 40,
            _ => 20,
        }
    }

    /// The launcher mode whose ranking the group uses.
    pub(crate) fn mode(self) -> &'static str {
        match self {
            Group::Settings => "actions",
            Group::Keys => "keys",
            Group::Computers => "tailscale",
            Group::Manual => "manual",
        }
    }
}

/// Rows a worker reads once, on the first search, and what the search
/// found among them.
#[derive(Default)]
pub(crate) struct Source {
    pub(crate) items: Option<Vec<Item>>,
    asked: bool,
    /// The rows found, best first.
    pub(crate) shown: Vec<usize>,
}

impl Source {
    /// True the first time: the caller starts the worker that reads it.
    pub(crate) fn ask(&mut self) -> bool {
        self.items.is_none() && !std::mem::replace(&mut self.asked, true)
    }

    /// Whether the rows are still being read.
    pub(crate) fn loading(&self) -> bool {
        self.asked && self.items.is_none()
    }

    pub(crate) fn filter(&mut self, query: &str, usage: &usage::Usage, group: Group) {
        self.shown.clear();
        let Some(items) = &self.items else { return };
        if query.trim().is_empty() {
            return;
        }
        let mut order = ranked_for_mode(items, query, usage, group.mode()).order;
        order.truncate(group.cap());
        self.shown = order;
    }

    pub(crate) fn get(&self, i: usize) -> Option<&Item> {
        self.items.as_ref()?.get(i)
    }
}

/// The view a `Target::Mode` row opens, as `nirulauncher --VIEW`.
pub(crate) fn view_flag(mode: &str) -> Option<String> {
    crate::VIEWS.contains(&mode).then(|| format!("--{mode}"))
}

/// Whether the panel can do what a row does: start a program, open a view
/// of the launcher, copy text or run a key binding. Rows that ask first
/// (logging out), only inform or need the launcher open stay in it.
pub(crate) fn runnable(target: &Target) -> bool {
    match target {
        Target::Spawn { .. } | Target::Terminal { .. } | Target::Copy(_) | Target::RunKey(_) => {
            true
        }
        Target::Mode(m) => view_flag(m).is_some(),
        _ => false,
    }
}

/// The settings and actions: the Actions view's static rows (no provider
/// is asked), the ones the panel can run, once each.
pub(crate) fn settings_items(cfg: &Config) -> Vec<Item> {
    keep_runnable(crate::modes::actions::action_static_items(cfg))
}

fn keep_runnable(items: Vec<Item>) -> Vec<Item> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|i| runnable(&i.target) && !i.key.starts_with("shortcut:"))
        .filter(|i| seen.insert(i.key.clone()))
        .collect()
}

/// The manual's chapters and headings (`nirux manual --path`), without the
/// row that says it is missing.
pub(crate) fn manual_items() -> Vec<Item> {
    crate::modes::manual::manual_items()
        .into_iter()
        .filter(|i| runnable(&i.target))
        .map(|mut i| {
            i.name = i.name.trim().to_string();
            i
        })
        .collect()
}

/// The key bindings in effect, without the row that says there are none.
pub(crate) fn key_items() -> Vec<Item> {
    crate::modes::keys::keys_items()
        .into_iter()
        .filter(|i| i.key != "shortcut:none")
        .collect()
}

/// The computers the panel shows, as rows to search: the name, the
/// address and the system. Row `k` is computer `k`.
pub(crate) fn computer_items(list: &[Device]) -> Vec<Item> {
    list.iter()
        .map(|d| Item {
            key: format!("computer:{}", d.name),
            group: String::new(),
            name: d.name.clone(),
            detail: d.ip.clone(),
            keywords: format!(
                "{} {} {} {}",
                d.host_name,
                d.target,
                d.os,
                if d.online { "online" } else { "offline" }
            ),
            favorite: false,
            target: Target::Info,
        })
        .collect()
}

/// How many rows each group gets in a column of `room` pixels: one row
/// each in turn, in order, while they fit. `costs[g]` are the heights of
/// group `g`'s rows; a group's first row also pays for its `header`.
pub(crate) fn allocate(costs: &[Vec<i32>], header: i32, room: i32) -> Vec<usize> {
    let mut take = vec![0; costs.len()];
    let mut used = 0;
    let mut full = vec![false; costs.len()];
    loop {
        let mut grew = false;
        for (g, rows) in costs.iter().enumerate() {
            if full[g] {
                continue;
            }
            let Some(&c) = rows.get(take[g]) else {
                full[g] = true;
                continue;
            };
            let c = c + if take[g] == 0 { header } else { 0 };
            if used + c <= room {
                used += c;
                take[g] += 1;
                grew = true;
            } else {
                // Rows stay in order: a group that cannot take its next
                // row takes no later one either.
                full[g] = true;
            }
        }
        if !grew {
            return take;
        }
    }
}

/// A result row's lines in `width` characters: the name on up to two
/// lines, what it is (`detail`) after it on the same line when it fits,
/// else on a line of its own when the name took one. Each line is
/// (name part, detail part).
pub(crate) fn row_lines(name: &str, detail: &str, width: usize) -> Vec<(String, String)> {
    let width = width.max(4);
    let mut lines: Vec<(String, String)> = super::wrap::wrap(name, width, 2)
        .into_iter()
        .map(|l| (l, String::new()))
        .collect();
    let detail = detail.trim();
    if detail.is_empty() {
        return lines;
    }
    let last = lines.last().map(|l| l.0.chars().count()).unwrap_or(0);
    let detail_chars = detail.chars().count();
    if last + 2 + detail_chars <= width {
        if let Some(l) = lines.last_mut() {
            l.1 = detail.to_string();
        }
    } else if lines.len() == 1 {
        let d = super::wrap::wrap(detail, width, 1).remove(0);
        lines.push((String::new(), d));
    } else if width > last + 6 {
        // A long name: what fits of the detail after it.
        let d = super::wrap::wrap(detail, width - last - 2, 1).remove(0);
        if let Some(l) = lines.last_mut() {
            l.1 = d;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, name: &str, target: Target) -> Item {
        Item {
            key: key.into(),
            group: "G".into(),
            name: name.into(),
            detail: String::new(),
            keywords: String::new(),
            favorite: false,
            target,
        }
    }

    #[test]
    fn rows_share_the_room_in_turn() {
        // Three groups of rows 20 px high, headers of 10.
        let costs = vec![vec![20; 5], vec![20; 1], vec![20; 3]];
        assert_eq!(allocate(&costs, 10, 0), [0, 0, 0]);
        assert_eq!(allocate(&costs, 10, 30), [1, 0, 0]);
        assert_eq!(allocate(&costs, 10, 90), [1, 1, 1]);
        assert_eq!(allocate(&costs, 10, 150), [3, 1, 2]);
        assert_eq!(allocate(&costs, 10, 1000), [5, 1, 3]);
        // A taller row that does not fit stops its group, not the others.
        let costs = vec![vec![20, 60, 20], vec![20, 20, 20]];
        assert_eq!(allocate(&costs, 0, 90), [1, 3]);
        assert_eq!(allocate(&costs, 0, 100), [2, 1]);
        assert!(allocate(&[], 10, 100).is_empty());
    }

    #[test]
    fn a_row_puts_its_detail_beside_or_below_the_name() {
        assert_eq!(
            row_lines("Displays", "Profiles", 40),
            [("Displays".to_string(), "Profiles".to_string())]
        );
        assert_eq!(
            row_lines("Displays", "Profiles, rotation and arrangement", 20),
            [
                ("Displays".to_string(), String::new()),
                (String::new(), "Profiles, rotation…".to_string())
            ]
        );
        let long = row_lines("A rather long name that needs two lines", "detail", 24);
        assert_eq!(long.len(), 2);
        assert!(long
            .iter()
            .all(|(n, d)| n.chars().count() + d.chars().count() < 24));
        assert_eq!(
            row_lines("Name", "", 10),
            [("Name".to_string(), String::new())]
        );
    }

    #[test]
    fn only_rows_the_panel_can_run_are_kept() {
        let spawn = || Target::Spawn {
            argv: vec!["true".into()],
            terminal: false,
            path: None,
        };
        let items = vec![
            row("a", "Spawn", spawn()),
            row("a", "Spawn again", spawn()),
            row("b", "A view", Target::Mode("themes".into())),
            row("c", "No view", Target::Mode("nowhere".into())),
            row("d", "Log out", Target::Confirm(crate::Action::Logout)),
            row("e", "Info", Target::Info),
            row("shortcut:mouse", "Mouse", spawn()),
            row("f", "Key", Target::RunKey("super+t".into())),
        ];
        let kept: Vec<String> = keep_runnable(items).into_iter().map(|i| i.name).collect();
        assert_eq!(kept, ["Spawn", "A view", "Key"]);
        assert_eq!(view_flag("clipboard").as_deref(), Some("--clipboard"));
        assert_eq!(view_flag("--x"), None);
    }

    #[test]
    fn a_source_finds_its_rows_and_is_asked_once() {
        let mut s = Source::default();
        assert!(s.ask());
        assert!(!s.ask());
        assert!(s.loading());
        let usage = usage::Usage::default();
        s.filter("disp", &usage, Group::Settings);
        assert!(s.shown.is_empty());
        let spawn = Target::Spawn {
            argv: vec!["true".into()],
            terminal: false,
            path: None,
        };
        s.items = Some(vec![
            row("x", "Displays", spawn.clone()),
            row("y", "Audio", spawn),
        ]);
        assert!(!s.loading());
        s.filter("disp", &usage, Group::Settings);
        assert_eq!(s.shown, [0]);
        s.filter("", &usage, Group::Settings);
        assert!(s.shown.is_empty());
    }
}
