//! `--apps --query TEXT --launch-best`: start the app a query clearly means.
//!
//! NIRUDICTATE's "open X" uses this. The apps are ranked exactly like the
//! Apps view ranks them (same index, favourites and frecency); the first row
//! is only started when the match is confident. Otherwise the Apps view
//! opens with the text typed, so a person picks.
//!
//! The confidence rule, on the query and names folded to lowercase letters
//! and digits without diacritics or spaces ("Niru Note" = "nirunote"):
//! 1. Every matching row gets a class: 0 the whole name, 1 a whole keyword
//!    or the start of the name, 2 the start of a name word or keyword,
//!    3 inside the name. Classes 1 to 3 need three characters or more.
//!    Rows that only matched loosely (letters in order, the group or detail)
//!    have no class and are never started.
//! 2. Only the best class counts. When one row has it, that row starts.
//! 3. When several share it, the first in the Apps view's order starts only
//!    if it is a favourite and the next is not, or its launch frecency is at
//!    least twice the next one's and at least one recent launch (1000).
//!    "terminal" with three terminal emulators and no history is therefore
//!    shown, not guessed.

use crate::*;

/// Frecency of one launch right now (see `usage::Usage::score`).
pub(crate) const MIN_SCORE: u32 = 1000;

/// What `--launch-best` does with a query.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Start this row (an index into the items).
    Launch(usize),
    /// Open the Apps view with the text typed.
    Show,
}

/// Lowercase letters and digits only, diacritics folded.
pub(crate) fn compact(text: &str) -> String {
    compose::fold(&text.to_lowercase())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// How the whole query matches one row; lower is better, None is loose.
pub(crate) fn class(item: &Item, query: &str) -> Option<u8> {
    let q = compact(query);
    if q.is_empty() {
        return None;
    }
    let name = compact(&item.name);
    if name == q {
        return Some(0);
    }
    let long = q.chars().count() >= 3;
    let keywords: Vec<String> = item
        .keywords
        .split(|c: char| c.is_whitespace() || c == ';' || c == ',')
        .map(compact)
        .filter(|k| !k.is_empty())
        .collect();
    if keywords.contains(&q) || (long && name.starts_with(&q)) {
        return Some(1);
    }
    if !long {
        return None;
    }
    let words = item
        .name
        .split(|c: char| !c.is_alphanumeric())
        .map(compact)
        .filter(|w| !w.is_empty());
    if words.chain(keywords).any(|w| w.starts_with(&q)) {
        return Some(2);
    }
    name.contains(&q).then_some(3)
}

/// Choose from rows already in the Apps view's order for this query.
pub(crate) fn choose(items: &[Item], order: &[usize], query: &str, usage: &usage::Usage) -> Pick {
    let classed: Vec<(u8, usize)> = order
        .iter()
        .filter_map(|&i| class(&items[i], query).map(|c| (c, i)))
        .collect();
    let Some(best) = classed.iter().map(|(c, _)| *c).min() else {
        return Pick::Show;
    };
    let tied: Vec<usize> = classed
        .iter()
        .filter(|(c, _)| *c == best)
        .map(|(_, i)| *i)
        .collect();
    let first = tied[0];
    let Some(&second) = tied.get(1) else {
        return Pick::Launch(first);
    };
    if items[first].favorite && !items[second].favorite {
        return Pick::Launch(first);
    }
    let (a, b) = (
        usage.score(&items[first].key),
        usage.score(&items[second].key),
    );
    if a >= MIN_SCORE && a >= b.saturating_mul(2) {
        Pick::Launch(first)
    } else {
        Pick::Show
    }
}

/// The desktop apps with favourites marked, ranked for the query.
fn apps_for(cfg: &Config, query: &str) -> (Vec<Item>, Vec<usize>, usage::Usage) {
    let mut items = app_items(desktop::discover());
    mark_favorites(&mut items, cfg);
    let usage = load_usage(cfg);
    let order = ranked_for_mode(&items, query, &usage, "apps").order;
    (items, order, usage)
}

/// `--apps --query TEXT [--launch-best]`.
pub(crate) fn run(query: &str, launch_best: bool) -> Result<()> {
    if launch_best && !query.trim().is_empty() {
        let cfg = Config::load()?;
        let (items, order, usage) = apps_for(&cfg, query);
        if let Pick::Launch(i) = choose(&items, &order, query, &usage) {
            let item = &items[i];
            if let Target::Spawn {
                argv,
                terminal,
                path,
            } = &item.target
            {
                spawn_command(argv, path.as_deref(), *terminal, &cfg)?;
                record_usage(&item.key);
                println!("launched\t{}", item.name);
                return Ok(());
            }
        }
    }
    let mut ui = Ui::new("apps")?;
    ui.query = query.to_string();
    ui.refilter();
    ui.draw()?;
    {
        use std::io::Write as _;
        let mut out = std::io::stdout();
        if launch_best {
            let _ = writeln!(out, "shown");
        }
        let _ = out.flush();
    }
    crate::run_ui(ui)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(key: &str, name: &str, keywords: &str) -> Item {
        Item {
            key: key.into(),
            group: "Applications".into(),
            name: name.into(),
            detail: key.into(),
            keywords: keywords.into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec![key.into()],
                terminal: false,
                path: None,
            },
        }
    }

    fn catalogue() -> Vec<Item> {
        vec![
            app(
                "kitty.desktop",
                "kitty",
                "Terminal emulator terminal;shell;",
            ),
            app(
                "Alacritty.desktop",
                "Alacritty",
                "Terminal A fast terminal emulator terminal;shell;",
            ),
            app(
                "xterm.desktop",
                "XTerm",
                "Terminal standard terminal emulator",
            ),
            app("nirunote.desktop", "NIRUNOTE", "Markdown notes editor"),
            app("niruwm-settings.desktop", "NIRUWM Settings", "settings"),
            app("firefox.desktop", "Firefox", "Web Browser internet;www;"),
            app(
                "org.gnome.Calculator.desktop",
                "Miniräknare",
                "kalkylator;räkna;",
            ),
            app("thunar.desktop", "Filhanterare", "Thunar filer;mappar;"),
            app("org.gnome.eog.desktop", "Bildvisare", "bilder;foton;"),
            app(
                "libreoffice-writer.desktop",
                "LibreOffice Writer",
                "Ordbehandlare text;dokument;",
            ),
            app("gimp.desktop", "GIMP", "Image Editor bild;"),
        ]
    }

    fn pick(items: &[Item], query: &str, usage: &usage::Usage) -> Option<String> {
        let order = ranked_for_mode(items, query, usage, "apps").order;
        match choose(items, &order, query, usage) {
            Pick::Launch(i) => Some(items[i].name.clone()),
            Pick::Show => None,
        }
    }

    #[test]
    fn exact_and_prefix_names_launch() {
        let items = catalogue();
        let none = usage::Usage::default();
        assert_eq!(pick(&items, "nirunote", &none).as_deref(), Some("NIRUNOTE"));
        assert_eq!(pick(&items, "NiruNote", &none).as_deref(), Some("NIRUNOTE"));
        // Whisper splits names into words.
        assert_eq!(
            pick(&items, "niru note", &none).as_deref(),
            Some("NIRUNOTE")
        );
        // A word cut short is still the start of the name.
        assert_eq!(pick(&items, "nirunot", &none).as_deref(), Some("NIRUNOTE"));
        assert_eq!(pick(&items, "firefox", &none).as_deref(), Some("Firefox"));
        assert_eq!(pick(&items, "fire", &none).as_deref(), Some("Firefox"));
        assert_eq!(pick(&items, "gimp", &none).as_deref(), Some("GIMP"));
        assert_eq!(
            pick(&items, "writer", &none).as_deref(),
            Some("LibreOffice Writer")
        );
        assert_eq!(pick(&items, "browser", &none).as_deref(), Some("Firefox"));
    }

    #[test]
    fn swedish_names_match_with_and_without_diacritics() {
        let items = catalogue();
        let none = usage::Usage::default();
        assert_eq!(
            pick(&items, "miniräknare", &none).as_deref(),
            Some("Miniräknare")
        );
        assert_eq!(
            pick(&items, "miniraknare", &none).as_deref(),
            Some("Miniräknare")
        );
        assert_eq!(
            pick(&items, "kalkylator", &none).as_deref(),
            Some("Miniräknare")
        );
        assert_eq!(
            pick(&items, "filhanterare", &none).as_deref(),
            Some("Filhanterare")
        );
        assert_eq!(
            pick(&items, "Bildvisare", &none).as_deref(),
            Some("Bildvisare")
        );
        assert_eq!(
            pick(&items, "ordbehandlare", &none).as_deref(),
            Some("LibreOffice Writer")
        );
    }

    #[test]
    fn several_terminals_are_shown_not_guessed() {
        let items = catalogue();
        let none = usage::Usage::default();
        assert_eq!(pick(&items, "terminal", &none), None);
        assert_eq!(pick(&items, "term", &none), None);
        // A favourite decides a tie.
        let mut fav = catalogue();
        fav[1].favorite = true;
        assert_eq!(pick(&fav, "terminal", &none).as_deref(), Some("Alacritty"));
        // So does clearly more use: three launches today against none.
        let now = 1_800_000_000;
        let used = usage::Usage::from_rows(
            [
                ("kitty.desktop", now),
                ("kitty.desktop", now),
                ("kitty.desktop", now - 60),
            ],
            now,
        );
        assert_eq!(pick(&items, "terminal", &used).as_deref(), Some("kitty"));
        // But not a near tie.
        let close = usage::Usage::from_rows(
            [
                ("kitty.desktop", now),
                ("kitty.desktop", now),
                ("Alacritty.desktop", now),
                ("Alacritty.desktop", now),
            ],
            now,
        );
        assert_eq!(pick(&items, "terminal", &close), None);
        // A better match class wins over a used tie: "xterm" is a name.
        assert_eq!(pick(&items, "xterm", &used).as_deref(), Some("XTerm"));
    }

    #[test]
    fn loose_and_short_queries_are_shown() {
        let items = catalogue();
        let none = usage::Usage::default();
        // Letters in order only.
        assert_eq!(pick(&items, "nrnt", &none), None);
        // A transposed typo matches nothing confidently.
        assert_eq!(pick(&items, "nirunoet", &none), None);
        // Too short to be sure, and nothing at all.
        assert_eq!(pick(&items, "fi", &none), None);
        assert_eq!(pick(&items, "spotify", &none), None);
        assert_eq!(pick(&items, "   ", &none), None);
        // Two apps start with "niru".
        assert_eq!(pick(&items, "niru", &none), None);
    }

    #[test]
    fn classes_follow_the_documented_rule() {
        let note = app("nirunote.desktop", "NIRUNOTE", "Markdown notes editor");
        assert_eq!(class(&note, "nirunote"), Some(0));
        assert_eq!(class(&note, "markdown"), Some(1));
        assert_eq!(class(&note, "niru"), Some(1));
        assert_eq!(class(&note, "mark"), Some(2));
        assert_eq!(class(&note, "note"), Some(2));
        assert_eq!(class(&note, "unot"), Some(3));
        assert_eq!(class(&note, "no"), None);
        assert_eq!(class(&note, "zzz"), None);
        assert_eq!(compact("Niru-Note Å"), "nirunotea");
    }
}
