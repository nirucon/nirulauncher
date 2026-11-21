//! The Demos view and the Play demo action: the fullscreen demos of
//! NIRUDEMO, read from `nirudemo --list`, played with `nirudemo NUMBER`.
//!
//! The list comes from NIRUDEMO itself, so a new demo in a later NIRUDEMO
//! shows up here without a launcher release. The query is bounded, so a
//! stuck or missing `nirudemo` never holds the launcher.
use crate::*;

/// A demo row: Enter plays it (`demo:play:NUMBER`).
pub(crate) const PLAY: &str = "demo:play:";
/// Play the default demo (`nirudemo` without arguments).
pub(crate) const PLAY_DEFAULT: &str = "demo:play";
/// The intent row for the same.
pub(crate) const PLAY_INTENT: &str = "intent:demo";
/// The row shown when NIRUDEMO is missing or does not answer.
pub(crate) const UNAVAILABLE: &str = "demo:unavailable";

/// How long `nirudemo --list` may take before the view gives up.
const LIST_BUDGET: Duration = Duration::from_millis(1500);

/// One line of `nirudemo --list`:
/// `NUMBER<TAB>KEY<TAB>TITLE<TAB>SECONDS<TAB>DESCRIPTION`.
#[derive(Debug, PartialEq)]
pub(crate) struct Demo {
    pub(crate) number: u32,
    pub(crate) key: String,
    pub(crate) title: String,
    pub(crate) seconds: u32,
    pub(crate) description: String,
}

/// Parse `nirudemo --list`. Lines without a number, key, title and length
/// are skipped; fields after the description are left for later releases.
pub(crate) fn parse_demos(text: &str) -> Vec<Demo> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim_end_matches('\r').split('\t');
            let number: u32 = parts.next()?.trim().parse().ok()?;
            let key = parts.next()?.trim();
            let title = parts.next()?.trim();
            let seconds: u32 = parts.next()?.trim().parse().ok()?;
            let description = parts.next().unwrap_or("").trim();
            (number > 0 && !key.is_empty() && !title.is_empty()).then(|| Demo {
                number,
                key: key.into(),
                title: title.into(),
                seconds,
                description: description.into(),
            })
        })
        .collect()
}

/// A length as `m:ss`.
pub(crate) fn length(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn unavailable_item() -> Item {
    Item {
        key: UNAVAILABLE.into(),
        group: "Demos".into(),
        name: "NIRUDEMO is not installed".into(),
        detail: "Run nirux update to install it".into(),
        keywords: "demo demos nirudemo".into(),
        favorite: false,
        target: Target::Info,
    }
}

pub(crate) fn items_from(demos: &[Demo]) -> Vec<Item> {
    if demos.is_empty() {
        return vec![unavailable_item()];
    }
    demos
        .iter()
        .map(|demo| Item {
            key: format!("{PLAY}{}", demo.number),
            group: "Demos".into(),
            name: demo.title.clone(),
            detail: if demo.description.is_empty() {
                length(demo.seconds)
            } else {
                format!("{} · {}", length(demo.seconds), demo.description)
            },
            keywords: format!("demo {} {} {}", demo.number, demo.key, demo.description),
            favorite: false,
            target: Target::Info,
        })
        .collect()
}

pub(crate) fn demos_items() -> Vec<Item> {
    let demos = match providers::bounded("nirudemo", &["--list"], LIST_BUDGET) {
        Some(out) if out.status.success() => parse_demos(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    };
    items_from(&demos)
}

/// The Play demo and Demos rows of NIRU Actions.
pub(crate) fn action_items() -> Vec<Item> {
    vec![
        Item {
            key: PLAY_DEFAULT.into(),
            group: "NIRU Actions".into(),
            name: "Play demo".into(),
            detail: "A fullscreen NIRUX demo with music; any key ends it · nirudemo".into(),
            keywords: "demo demos nirudemo demoscene play spela visa".into(),
            favorite: false,
            target: Target::Info,
        },
        Item {
            key: "action:demos".into(),
            group: "NIRU Actions".into(),
            name: "Demos".into(),
            detail: "Every NIRUDEMO demo with its length; Enter plays one".into(),
            keywords: "demos demo nirudemo demoscene list alla".into(),
            favorite: false,
            target: Target::Mode("demos".into()),
        },
    ]
}

/// The `nirudemo` argv for Enter on a row, or `None` when the row is not a
/// demo to play.
pub(crate) fn play_argv(key: &str) -> Option<Vec<String>> {
    if key == PLAY_DEFAULT || key == PLAY_INTENT {
        return Some(vec!["nirudemo".into()]);
    }
    let number: u32 = key.strip_prefix(PLAY)?.parse().ok()?;
    Some(vec!["nirudemo".into(), number.to_string()])
}

impl Ui {
    /// Enter on a demo row. `None` means it was not one.
    pub(crate) fn demo_execute(&mut self, key: &str) -> Result<Option<bool>> {
        let Some(argv) = play_argv(key) else {
            return Ok(None);
        };
        if !in_path("nirudemo") {
            self.error = "NIRUDEMO is not installed; run nirux update".into();
            self.draw()?;
            return Ok(Some(true));
        }
        // Close first: the demo grabs the keyboard and the pointer, which
        // fails while the launcher still holds them.
        self.close()?;
        let _ = self.c.get_input_focus()?.reply();
        match spawn_command(&argv, None, false, &self.cfg) {
            Ok(()) => record_usage(key),
            Err(e) => eprintln!("nirulauncher: {e}"),
        }
        Ok(Some(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn good_lines_parse_in_order() {
        let text = "1\tginnungagap\tGinnungagap\t107\tStars, a low-poly NIRUX logo in fog, plasma and a sine scroller\n2\tyggdrasil\tYggdrasil\t65\tA tree\n";
        let demos = parse_demos(text);
        assert_eq!(demos.len(), 2);
        assert_eq!(
            demos[0],
            Demo {
                number: 1,
                key: "ginnungagap".into(),
                title: "Ginnungagap".into(),
                seconds: 107,
                description: "Stars, a low-poly NIRUX logo in fog, plasma and a sine scroller"
                    .into(),
            }
        );
        assert_eq!(demos[1].number, 2);
        assert_eq!(demos[1].title, "Yggdrasil");
    }

    #[test]
    fn bad_lines_are_skipped() {
        let text = "\nnot a demo\nx\tkey\tTitle\t10\tbad number\n3\tkey\tTitle\tlong\tbad length\n0\tzero\tZero\t5\tnumber zero\n4\t\tNo key\t5\t\n5\tnotitle\t\t5\tx\n6\tshort\tShort\n-1\tneg\tNeg\t5\tx\n7\tok\tOk\t9\n";
        let demos = parse_demos(text);
        assert_eq!(demos.len(), 1, "{demos:?}");
        assert_eq!(demos[0].key, "ok");
        assert_eq!(demos[0].description, "");
    }

    #[test]
    fn extra_fields_and_crlf_are_ignored() {
        let demos = parse_demos("2\tkey\tTitle\t61\tDescribed\tfuture\tfields\r\n");
        assert_eq!(demos.len(), 1);
        assert_eq!(demos[0].description, "Described");
        assert_eq!(demos[0].seconds, 61);
    }

    #[test]
    fn empty_output_shows_the_missing_program_row() {
        assert!(parse_demos("").is_empty());
        let items = items_from(&parse_demos(""));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, UNAVAILABLE);
        assert_eq!(items[0].name, "NIRUDEMO is not installed");
        assert!(items[0].detail.contains("nirux update"));
        assert!(matches!(items[0].target, Target::Info));
        assert!(play_argv(UNAVAILABLE).is_none());
    }

    #[test]
    fn lengths_are_minutes_and_two_digit_seconds() {
        assert_eq!(length(0), "0:00");
        assert_eq!(length(7), "0:07");
        assert_eq!(length(60), "1:00");
        assert_eq!(length(107), "1:47");
        assert_eq!(length(3599), "59:59");
        assert_eq!(length(3600), "60:00");
    }

    #[test]
    fn rows_show_title_length_and_description_and_play_by_number() {
        let items = items_from(&parse_demos(
            "1\tginnungagap\tGinnungagap\t107\tStars and plasma\n12\tnew\tNew\t5\t\n",
        ));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "Ginnungagap");
        assert_eq!(items[0].detail, "1:47 · Stars and plasma");
        assert_eq!(items[1].detail, "0:05");
        assert_eq!(play_argv(&items[0].key).unwrap(), ["nirudemo", "1"]);
        assert_eq!(play_argv(&items[1].key).unwrap(), ["nirudemo", "12"]);
        assert_eq!(play_argv(PLAY_DEFAULT).unwrap(), ["nirudemo"]);
        assert_eq!(play_argv(PLAY_INTENT).unwrap(), ["nirudemo"]);
        assert!(play_argv("demo:play:1; rm").is_none());
        assert!(play_argv("action:demos").is_none());
    }

    /// `demos_items` with only `timeout` and, when given, a fake
    /// `nirudemo` running `script` on PATH, and HOME in the same sandbox.
    fn items_with(name: &str, script: Option<&str>) -> (Vec<Item>, Duration) {
        use std::os::unix::fs::PermissionsExt;
        let _env = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("nl-demo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let timeout = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .chain(["/usr/bin".into(), "/bin".into()])
            .map(|d| d.join("timeout"))
            .find(|p| p.is_file())
            .expect("timeout from coreutils");
        std::os::unix::fs::symlink(timeout, dir.join("timeout")).unwrap();
        if let Some(script) = script {
            let fake = dir.join("nirudemo");
            std::fs::write(&fake, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::var_os("PATH");
        let home = std::env::var_os("HOME");
        std::env::set_var("PATH", &dir);
        std::env::set_var("HOME", &dir);
        providers::forget();
        let start = Instant::now();
        let items = demos_items();
        let took = start.elapsed();
        providers::forget();
        match path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        match home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        (items, took)
    }

    #[test]
    fn a_missing_nirudemo_shows_the_missing_program_row() {
        let (items, _) = items_with("missing", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, UNAVAILABLE);
    }

    #[test]
    fn a_failing_or_stuck_nirudemo_shows_the_missing_program_row() {
        let (items, _) = items_with("fails", Some("echo 'nirudemo: unknown option' >&2; exit 2"));
        assert_eq!(items[0].key, UNAVAILABLE);
        let (items, took) = items_with("stuck", Some("exec /bin/sleep 10"));
        assert_eq!(items[0].key, UNAVAILABLE);
        assert!(took < Duration::from_secs(4), "took {took:?}");
    }

    #[test]
    fn a_fake_nirudemo_lists_its_demos() {
        let (items, _) = items_with(
            "list",
            Some("[ \"$1\" = --list ] || exit 2\nprintf '1\\tginnungagap\\tGinnungagap\\t107\\tStars\\n2\\tnext\\tNext\\t75\\tMore\\n'"),
        );
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Ginnungagap", "Next"]);
        assert_eq!(items[1].detail, "1:15 · More");
    }
}
