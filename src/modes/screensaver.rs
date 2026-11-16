//! The Screensaver view: which scene NIRUSCREENSAVER shows, which scenes
//! `shuffle` picks from, a preview, and the settings that shape them.
//!
//! Every row reads and changes `~/.config/niruui/screensaver.toml` through
//! NIRUSCREENSAVER (`--list`, `--settings`, `--set`, `--shuffle`), so the
//! file stays the only description: comments are kept and a bad value is
//! refused before anything is written. The next time the screensaver
//! appears it reads the file; nothing needs a restart.
use crate::*;

/// A scene row: Enter chooses it.
pub(crate) const SCENE: &str = "saver:scene:";
/// A shuffle row: Enter adds or removes the scene.
pub(crate) const SHUFFLE: &str = "saver:shuffle:";
/// A setting row: Enter steps to the next value.
pub(crate) const SETTING: &str = "saver:set:";
/// Show the chosen scene now on this monitor.
pub(crate) const PREVIEW: &str = "saver:preview";

/// One line of `niruscreensaver --list`.
#[derive(Debug, PartialEq)]
pub(crate) struct Scene {
    pub(crate) name: String,
    pub(crate) chosen: bool,
    pub(crate) shuffle: bool,
    pub(crate) description: String,
}

pub(crate) fn parse_scenes(text: &str) -> Vec<Scene> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let name = parts.next()?.trim();
            let chosen = parts.next()? == "on";
            let shuffle = parts.next()? == "shuffle";
            let description = parts.next().unwrap_or("").trim();
            (!name.is_empty()).then(|| Scene {
                name: name.into(),
                chosen,
                shuffle,
                description: description.into(),
            })
        })
        .collect()
}

/// `key<TAB>value` lines of `niruscreensaver --settings`.
pub(crate) fn parse_settings(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// The values Enter steps through for a setting row.
fn cycle(key: &str) -> &'static [&'static str] {
    match key {
        "layout" => &["cava", "minimal", "terminal", "runic", "corner"],
        "visual" => &["auto", "cava", "quotes", "none"],
        "photos_seconds" => &["10", "20", "30", "60", "120"],
        _ => &["true", "false"],
    }
}

/// The value after `current` in the row's cycle.
pub(crate) fn next_value(key: &str, current: &str) -> String {
    let values = cycle(key);
    let at = values.iter().position(|v| *v == current);
    values[at.map_or(0, |i| (i + 1) % values.len())].to_string()
}

fn item(key: String, group: &str, name: String, detail: String, keywords: String) -> Item {
    Item {
        key,
        group: group.into(),
        name,
        detail,
        keywords,
        favorite: false,
        target: Target::Info,
    }
}

fn unavailable(detail: String) -> Vec<Item> {
    vec![item(
        "saver:unavailable".into(),
        "Screensaver",
        "The screensaver cannot be set up here".into(),
        detail,
        "screensaver".into(),
    )]
}

/// Run NIRUSCREENSAVER. Reads go through the provider cache, which a
/// change clears first; a change itself always runs.
fn run(args: &[&str]) -> std::result::Result<String, String> {
    let read = matches!(args, ["--settings"] | ["--list"]);
    let result = if read {
        providers::output("niruscreensaver", args)
    } else {
        providers::forget();
        Command::new("niruscreensaver")
            .args(args)
            .stdin(Stdio::null())
            .output()
    };
    match result {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stderr);
            // The first line; the usage text after it is for terminals.
            let first = text.lines().next().unwrap_or("").trim().to_string();
            Err(if first.is_empty() {
                format!("niruscreensaver {} failed", args.join(" "))
            } else {
                first.trim_start_matches("niruscreensaver: ").to_string()
            })
        }
        Err(e) => Err(format!("could not start niruscreensaver: {e}")),
    }
}

pub(crate) fn screensaver_items() -> Vec<Item> {
    let scenes = match run(&["--list"]) {
        Ok(text) => parse_scenes(&text),
        Err(e) if e.contains("Usage") || e.contains("unknown") => {
            return unavailable("This NIRUSCREENSAVER is too old; run nirux update".into())
        }
        Err(e) => return unavailable(e),
    };
    if scenes.is_empty() {
        return unavailable("This NIRUSCREENSAVER is too old; run nirux update".into());
    }
    let settings = run(&["--settings"])
        .map(|text| parse_settings(&text))
        .unwrap_or_default();
    items_from(&scenes, &settings)
}

pub(crate) fn items_from(scenes: &[Scene], settings: &[(String, String)]) -> Vec<Item> {
    let value = |key: &str| {
        settings
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    };
    let mut items = Vec::new();
    let chosen = scenes.iter().find(|s| s.chosen);
    for scene in chosen
        .into_iter()
        .chain(scenes.iter().filter(|s| !s.chosen))
    {
        items.push(item(
            format!("{SCENE}{}", scene.name),
            if scene.chosen {
                "Shown now"
            } else {
                "Other scenes"
            },
            scene.name.clone(),
            scene.description.clone(),
            format!("screensaver scene {} {}", scene.name, scene.description),
        ));
        if scene.chosen {
            items.push(item(
                PREVIEW.into(),
                "Shown now",
                format!("Preview {}", scene.name),
                "Show it now on this monitor; any key ends it".into(),
                "screensaver preview try show".into(),
            ));
        }
    }
    for scene in scenes.iter().filter(|s| s.name != "shuffle") {
        items.push(item(
            format!("{SHUFFLE}{}", scene.name),
            "In shuffle",
            format!(
                "{}: {}",
                scene.name,
                if scene.shuffle { "yes" } else { "no" }
            ),
            "Whether shuffle may pick it".into(),
            format!("screensaver shuffle {}", scene.name),
        ));
    }
    let mut setting = |key: &str, label: &str, detail: &str| {
        let current = value(key);
        if current.is_empty() {
            return;
        }
        let shown = match current {
            "true" => "on",
            "false" => "off",
            other => other,
        };
        let shown = if key == "photos_seconds" {
            format!("{shown} s")
        } else {
            shown.to_string()
        };
        items.push(item(
            format!("{SETTING}{key}"),
            "Settings",
            format!("{label}: {shown}"),
            detail.into(),
            format!("screensaver setting {key} {label}"),
        ));
    };
    setting(
        "layout",
        "Clock layout",
        "Enter steps through cava, minimal, terminal, runic and corner",
    );
    setting(
        "visual",
        "Clock panel",
        "auto: the spectrum while music plays, otherwise a quote",
    );
    setting(
        "show_media",
        "Track line",
        "Artist and title under the spectrum",
    );
    setting(
        "photos_seconds",
        "Seconds per photo",
        "Enter steps through 10, 20, 30, 60 and 120",
    );
    setting(
        "reduced_motion",
        "Reduced motion",
        "No fades or drift; the dark line moves once a minute",
    );
    let folder = value("photos_folder");
    if !folder.is_empty() {
        items.push(item(
            "saver:folder".into(),
            "Settings",
            format!("Photo folder: {folder}"),
            "Change it with niruscreensaver --set photos_folder PATH".into(),
            "screensaver photos folder pictures".into(),
        ));
    }
    items
}

/// The `niruscreensaver` arguments for Enter on a row, or `None` for a row
/// that changes nothing.
pub(crate) fn enter_args(key: &str, items: &[Item]) -> Option<Vec<String>> {
    if let Some(name) = key.strip_prefix(SCENE) {
        return Some(vec!["--set".into(), "scene".into(), name.into()]);
    }
    if let Some(name) = key.strip_prefix(SHUFFLE) {
        return Some(vec!["--shuffle".into(), "toggle".into(), name.into()]);
    }
    let setting = key.strip_prefix(SETTING)?;
    let name = &items.iter().find(|item| item.key == key)?.name;
    let current = name.rsplit(": ").next()?.trim_end_matches(" s");
    let current = match current {
        "on" => "true",
        "off" => "false",
        other => other,
    };
    Some(vec![
        "--set".into(),
        setting.into(),
        next_value(setting, current),
    ])
}

impl Ui {
    /// Enter on a Screensaver row. `None` means it was not one.
    pub(crate) fn screensaver_execute(&mut self, key: &str) -> Result<Option<bool>> {
        if key == PREVIEW {
            // Close first: the preview grabs the keyboard and the pointer,
            // which fails while the launcher still holds them.
            self.close()?;
            let _ = self.c.get_input_focus()?.reply();
            let argv = vec!["niruscreensaver".to_string(), "--preview".into()];
            if let Err(e) = spawn_command(&argv, None, false, &self.cfg) {
                eprintln!("nirulauncher: {e}");
            }
            return Ok(Some(false));
        }
        let Some(args) = enter_args(key, &self.items) else {
            return Ok(None);
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.error = match run(&args) {
            Ok(_) => String::new(),
            Err(e) => e,
        };
        self.items = screensaver_items();
        self.plain_items = self.items.len();
        self.filtered = filtered_for_mode(&self.items, &self.query, &self.usage, &self.mode);
        // The selection follows the row: a chosen scene moves to the top.
        self.selected = self
            .filtered
            .iter()
            .position(|&i| self.items[i].key == key)
            .unwrap_or(self.selected)
            .min(self.filtered.len().saturating_sub(1));
        self.reveal_selection();
        self.draw()?;
        Ok(Some(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "clock\toff\tshuffle\tLarge clock\nphotos\toff\tshuffle\tSlide show\ndark\ton\tshuffle\tBlack\ndark-sid\toff\t-\tAs dark, with SID\nshuffle\toff\t-\tAnother scene\n";
    const SETTINGS: &str = "scene\tdark\nlayout\tcava\nvisual\tauto\nshuffle\tclock,photos,dark\nphotos_folder\t~/Pictures/Wallpapers/NIRU\nphotos_seconds\t20\nshow_media\ttrue\nreduced_motion\tfalse\n";

    #[test]
    fn the_chosen_scene_comes_first_with_a_preview() {
        let items = items_from(&parse_scenes(LIST), &parse_settings(SETTINGS));
        assert_eq!(items[0].key, "saver:scene:dark");
        assert_eq!(items[0].group, "Shown now");
        assert_eq!(items[1].key, PREVIEW);
        assert_eq!(items[2].key, "saver:scene:clock");
        assert!(items.iter().any(|i| i.name == "dark-sid: no"));
        assert!(!items.iter().any(|i| i.key == "saver:shuffle:shuffle"));
        assert!(items.iter().any(|i| i.name == "Seconds per photo: 20 s"));
        assert!(items.iter().any(|i| i.name == "Reduced motion: off"));
    }

    #[test]
    fn enter_chooses_toggles_and_steps() {
        let items = items_from(&parse_scenes(LIST), &parse_settings(SETTINGS));
        let args = |key: &str| enter_args(key, &items).map(|a| a.join(" "));
        assert_eq!(args("saver:scene:photos").unwrap(), "--set scene photos");
        assert_eq!(
            args("saver:shuffle:dark-sid").unwrap(),
            "--shuffle toggle dark-sid"
        );
        assert_eq!(args("saver:set:layout").unwrap(), "--set layout minimal");
        assert_eq!(
            args("saver:set:photos_seconds").unwrap(),
            "--set photos_seconds 30"
        );
        assert_eq!(
            args("saver:set:reduced_motion").unwrap(),
            "--set reduced_motion true"
        );
        assert_eq!(
            args("saver:set:show_media").unwrap(),
            "--set show_media false"
        );
        assert!(args("saver:folder").is_none());
        assert!(args(PREVIEW).is_none());
    }

    #[test]
    fn values_cycle_and_unknown_ones_restart() {
        assert_eq!(next_value("layout", "corner"), "cava");
        assert_eq!(next_value("visual", "none"), "auto");
        assert_eq!(next_value("photos_seconds", "45"), "10");
        assert_eq!(next_value("show_media", "true"), "false");
    }
}
