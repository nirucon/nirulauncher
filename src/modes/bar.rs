//! The Bar view: which modules the bar shows, in which order, and a few bar
//! settings.
//!
//! Every row reads and changes the configuration file through NIRUBAR
//! (`nirubar --modules ...`, `--settings`, `--set`), so the file stays the
//! only description of the bar: comments are kept, the old file goes into
//! NIRUWM's settings history and the bars reload at once. Enter turns a module
//! on or off; Alt+Up and Alt+Down move the selected module.
use crate::*;

/// The key prefix of a module row.
pub(crate) const MODULE: &str = "bar:module:";
/// The key prefix of a setting row.
pub(crate) const SETTING: &str = "bar:set:";
/// Put the NIRUX list back and follow it again.
pub(crate) const DEFAULT: &str = "bar:default";

/// One line of `nirubar --modules all`.
#[derive(Debug, PartialEq)]
pub(crate) struct Module {
    pub(crate) name: String,
    pub(crate) on: bool,
    pub(crate) description: String,
}

pub(crate) fn parse_modules(text: &str) -> Vec<Module> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let name = parts.next()?.trim();
            let on = parts.next()? == "on";
            let description = parts.next().unwrap_or("").trim();
            (!name.is_empty()).then(|| Module {
                name: name.into(),
                on,
                description: description.into(),
            })
        })
        .collect()
}

/// `key<TAB>value` lines of `nirubar --settings`.
pub(crate) fn parse_settings(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn unavailable(detail: String) -> Vec<Item> {
    vec![Item {
        key: "bar:unavailable".into(),
        group: "Bar".into(),
        name: "The bar cannot be arranged here".into(),
        detail,
        keywords: "bar modules".into(),
        favorite: false,
        target: Target::Info,
    }]
}

/// Run NIRUBAR. Reads go through the provider cache, which a change clears
/// first; a change itself always runs.
fn run(args: &[&str]) -> std::result::Result<String, String> {
    let read = matches!(args, ["--settings"] | ["--modules", "all"]);
    let result = if read {
        providers::output("nirubar", args)
    } else {
        providers::forget();
        Command::new("nirubar")
            .args(args)
            .stdin(Stdio::null())
            .output()
    };
    match result {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if text.is_empty() {
                format!("nirubar {} failed", args.join(" "))
            } else {
                text.trim_start_matches("nirubar: ").to_string()
            })
        }
        Err(e) => Err(format!("could not start nirubar: {e}")),
    }
}

pub(crate) fn bar_items() -> Vec<Item> {
    let modules = match run(&["--modules", "all"]) {
        Ok(text) => parse_modules(&text),
        Err(e) if e.contains("Usage") || e.contains("unknown") => {
            return unavailable("This NIRUBAR is too old; run nirux update".into())
        }
        Err(e) => return unavailable(e),
    };
    let settings = run(&["--settings"])
        .map(|text| parse_settings(&text))
        .unwrap_or_default();
    items_from(&modules, &settings)
}

pub(crate) fn items_from(modules: &[Module], settings: &[(String, String)]) -> Vec<Item> {
    let on: Vec<&Module> = modules.iter().filter(|m| m.on).collect();
    let mut items = Vec::new();
    for (index, module) in on.iter().enumerate() {
        items.push(Item {
            key: format!("{MODULE}{}", module.name),
            group: "On the bar".into(),
            name: format!("{:>2}  {}", index + 1, module.name),
            detail: module.description.clone(),
            keywords: format!("bar module on {} {}", module.name, module.description),
            favorite: false,
            target: Target::Info,
        });
    }
    for module in modules.iter().filter(|m| !m.on) {
        items.push(Item {
            key: format!("{MODULE}{}", module.name),
            group: "Not on the bar".into(),
            name: format!("    {}", module.name),
            detail: module.description.clone(),
            keywords: format!("bar module off {} {}", module.name, module.description),
            favorite: false,
            target: Target::Info,
        });
    }
    let value = |key: &str| {
        settings
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let switch = |key: &str, label: &str, detail: &str| {
        value(key).map(|v| Item {
            key: format!("{SETTING}{key}"),
            group: "Bar settings".into(),
            name: format!("{label}: {}", if v == "true" { "on" } else { "off" }),
            detail: detail.into(),
            keywords: format!("bar setting {key} {label}"),
            favorite: false,
            target: Target::Info,
        })
    };
    if let Some(position) = value("position") {
        items.push(Item {
            key: format!("{SETTING}position"),
            group: "Bar settings".into(),
            name: format!("Position: {position}"),
            detail: "Enter moves the bar to the other edge".into(),
            keywords: "bar setting position top bottom edge".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    items.extend(switch("icons", "Icons", "Icons instead of short words"));
    items.extend(switch(
        "title",
        "Window title",
        "The focused window's title next to the workspaces",
    ));
    items.extend(switch("show_date", "Date", "The date in the clock"));
    items.extend(switch("show_time", "Time", "The time in the clock"));
    items.extend(switch("tray", "System tray", "Icons of apps that have one"));
    items.extend(switch(
        "popups",
        "Panels on click",
        "A click on a module opens its panel",
    ));
    let following = value("follow_default_modules") == Some("true");
    items.push(Item {
        key: DEFAULT.into(),
        group: "Bar settings".into(),
        name: if following {
            "Modules follow the NIRUX default".into()
        } else {
            "Use the NIRUX default modules".into()
        },
        detail: if following {
            "Updates bring new modules; any change here keeps your own list".into()
        } else {
            "Your own list is kept; Enter puts the NIRUX list back".into()
        },
        keywords: "bar modules default reset nirux".into(),
        favorite: false,
        target: Target::Info,
    });
    items
}

/// The `nirubar` arguments for Enter on a row, or `None` for another row.
pub(crate) fn enter_args(key: &str, items: &[Item]) -> Option<Vec<String>> {
    if let Some(name) = key.strip_prefix(MODULE) {
        return Some(vec!["--modules".into(), "toggle".into(), name.into()]);
    }
    if key == DEFAULT {
        return Some(vec!["--modules".into(), "default".into()]);
    }
    let setting = key.strip_prefix(SETTING)?;
    let name = items.iter().find(|item| item.key == key)?.name.clone();
    let value = if setting == "position" {
        if name.ends_with("bottom") {
            "top"
        } else {
            "bottom"
        }
    } else if name.ends_with(": on") {
        "false"
    } else {
        "true"
    };
    Some(vec!["--set".into(), setting.into(), value.into()])
}

impl Ui {
    /// Enter on a Bar row. `None` means it was not one.
    pub(crate) fn bar_execute(&mut self, key: &str) -> Result<Option<bool>> {
        let Some(args) = enter_args(key, &self.items) else {
            return Ok(None);
        };
        // On or off: the selection stays on the same row, so several modules
        // can be switched one after another.
        self.bar_change(&args, None)?;
        Ok(Some(true))
    }

    /// Alt+Up / Alt+Down on a module that is on the bar.
    pub(crate) fn bar_move(&mut self, up: bool) -> Result<()> {
        let Some(&index) = self.filtered.get(self.selected) else {
            return Ok(());
        };
        let key = self.items[index].key.clone();
        let Some(name) = key.strip_prefix(MODULE) else {
            return Ok(());
        };
        if !self.items[index].group.starts_with("On the bar") {
            self.error = "Add the module first (Enter), then move it".into();
            return self.draw();
        }
        let args = [
            "--modules".to_string(),
            "move".into(),
            name.into(),
            if up { "up" } else { "down" }.into(),
        ];
        // A move: the selection follows the module.
        self.bar_change(&args, Some(&key))
    }

    fn bar_change(&mut self, args: &[String], follow: Option<&str>) -> Result<()> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.error = match run(&args) {
            Ok(_) => String::new(),
            Err(e) => e,
        };
        self.items = bar_items();
        self.plain_items = self.items.len();
        self.filtered = filtered_for_mode(&self.items, &self.query, &self.usage, &self.mode);
        let row = match follow {
            Some(key) => self
                .filtered
                .iter()
                .position(|&i| self.items[i].key == key)
                .unwrap_or(0),
            None => self.selected,
        };
        self.selected = row.min(self.filtered.len().saturating_sub(1));
        self.reveal_selection();
        self.draw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULES: &str =
        "workspaces\ton\tWorkspace cells\nclock\ton\tDate and time\npulse\toff\tCPU and memory\n";
    const SETTINGS: &str = "position\ttop\ntitle\tfalse\nicons\ttrue\nshow_date\ttrue\nshow_time\ttrue\ntray\ttrue\npopups\ttrue\nfollow_default_modules\ttrue\n";

    #[test]
    fn modules_on_the_bar_come_first_and_in_order() {
        let items = items_from(&parse_modules(MODULES), &parse_settings(SETTINGS));
        assert_eq!(items[0].key, "bar:module:workspaces");
        assert!(items[0].name.contains("1  workspaces"));
        assert_eq!(items[1].key, "bar:module:clock");
        assert_eq!(items[2].key, "bar:module:pulse");
        assert!(items[2].group.starts_with("Not on the bar"));
        assert!(items.iter().any(|i| i.name == "Position: top"));
        assert!(items.iter().any(|i| i.name == "Window title: off"));
        assert!(items.iter().any(|i| i.key == DEFAULT));
    }

    #[test]
    fn enter_toggles_modules_and_flips_settings() {
        let items = items_from(&parse_modules(MODULES), &parse_settings(SETTINGS));
        let args = |key: &str| enter_args(key, &items).unwrap().join(" ");
        assert_eq!(args("bar:module:clock"), "--modules toggle clock");
        assert_eq!(args("bar:module:pulse"), "--modules toggle pulse");
        assert_eq!(args("bar:set:position"), "--set position bottom");
        assert_eq!(args("bar:set:title"), "--set title true");
        assert_eq!(args("bar:set:icons"), "--set icons false");
        assert_eq!(args(DEFAULT), "--modules default");
        assert!(enter_args("settings:themes", &items).is_none());
    }

    #[test]
    fn broken_lines_are_skipped() {
        assert_eq!(parse_modules("\nclock\n\tonly tabs\n").len(), 0);
        assert_eq!(parse_modules("clock\ton").len(), 1);
    }
}
