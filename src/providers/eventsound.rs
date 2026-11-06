//! Event and per-app notification sounds, through `nirux sounds` only.
//!
//! The launcher owns no sound configuration and parses no TOML: it reads
//! `nirux sounds status --json` (schema 1) and changes things with the same
//! `nirux sounds …` argv a user would type. Every call runs in a worker with a
//! time limit; nothing goes through a shell.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The events an older NIRUX (no categories in its status) knows.
pub const LEGACY_EVENTS: [&str; 4] = ["startup", "notification", "update", "shutdown"];

/// NIRUX sound categories, in the order they are shown. A category NIRUX
/// reports that is not listed here is shown after these.
pub const CATEGORY_ORDER: [&str; 5] = ["system", "notifications", "feedback", "power", "devices"];

/// A `nirux sounds` call must not keep the Sound view busy for long.
const BUDGET_SECS: &str = "5";

/// Every event NIRUX reports, by name, with whether it is on.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Events(pub BTreeMap<String, bool>);

impl Events {
    pub fn get(&self, event: &str) -> bool {
        self.0.get(event).copied().unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Category {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct App {
    pub app: String,
    #[serde(default)]
    pub sound: String,
    /// `ok`, `missing` (plays the notification sound), `silent` or `default`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub file: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Status {
    pub schema: u32,
    pub enabled: bool,
    #[serde(default)]
    pub volume: u32,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub events: Events,
    /// Present from NIRUX 0.12.219; empty on older versions.
    #[serde(default)]
    pub categories: BTreeMap<String, Category>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub apps: Vec<App>,
}

/// One shown category: its name and whether it is on (`None` for an older
/// NIRUX without categories), then its events.
pub type Group = (Option<(String, bool)>, Vec<String>);

impl Status {
    /// The categories to show, with their events, in display order. An older
    /// NIRUX without categories yields one unnamed group of its four events.
    pub fn groups(&self) -> Vec<Group> {
        if self.categories.is_empty() {
            return vec![(None, LEGACY_EVENTS.iter().map(|e| e.to_string()).collect())];
        }
        let mut names: Vec<&String> = self.categories.keys().collect();
        let rank = |name: &str| {
            CATEGORY_ORDER
                .iter()
                .position(|c| *c == name)
                .unwrap_or(usize::MAX)
        };
        names.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
        names
            .into_iter()
            .map(|name| {
                let category = &self.categories[name];
                (
                    Some((name.clone(), category.enabled)),
                    category.events.clone(),
                )
            })
            .collect()
    }
}

/// Parse `nirux sounds status --json`. Anything but schema 1 is treated as
/// a NIRUX that cannot describe its sounds to the launcher.
pub fn parse(bytes: &[u8]) -> Result<Status, String> {
    let status: Status = serde_json::from_slice(bytes)
        .map_err(|_| "This NIRUX has no sound status; update NIRUX".to_string())?;
    if status.schema != 1 {
        return Err(format!(
            "Unknown NIRUX sound status schema {}",
            status.schema
        ));
    }
    Ok(status)
}

/// `nirux ARGS…` with a time limit. The answer is stdout on success.
fn nirux(args: &[String]) -> Result<String, String> {
    let output = Command::new("timeout")
        .args(["-k", "0.5", BUDGET_SECS, "nirux"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not start nirux: {error}"))?;
    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).trim().to_string()),
        Some(124) => Err("nirux did not answer in time".into()),
        // GNU timeout reports a program it cannot start as 126/127.
        Some(126 | 127) => Err("NIRUX is not installed".into()),
        _ => {
            let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(text
                .lines()
                .last()
                .map(|line| line.trim_start_matches("nirux: ").to_string())
                .filter(|line| !line.is_empty())
                .unwrap_or_else(|| "nirux rejected the request".into()))
        }
    }
}

/// Read the current state.
pub fn status() -> Result<Status, String> {
    let output = Command::new("timeout")
        .args([
            "-k",
            "0.5",
            BUDGET_SECS,
            "nirux",
            "sounds",
            "status",
            "--json",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("could not start nirux: {error}"))?;
    match output.status.code() {
        Some(0) => parse(&output.stdout),
        Some(124) => Err("nirux sounds status did not answer in time".into()),
        Some(126 | 127) => Err("NIRUX is not installed".into()),
        // An older NIRUX rejects --json with its usage text.
        _ => Err("This NIRUX has no sound status; update NIRUX".into()),
    }
}

/// Run one change, then read the state again. The message is NIRUX's own
/// confirmation line (`signal = chat`).
pub fn change(args: Vec<String>) -> (Result<Status, String>, Result<String, String>) {
    let answer = nirux(&args).map(|text| text.lines().last().unwrap_or("").to_string());
    (status(), answer)
}

/// The same rule as `nirux sounds app`: letters, digits, `.`, `_` and `-`.
pub fn valid_app(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// The key NIRUX stores an app under (`Thunderbird.desktop` → `thunderbird`).
pub fn app_key(name: &str) -> String {
    let name = name.trim().to_lowercase();
    name.strip_suffix(".desktop").unwrap_or(&name).to_string()
}

/// A sound name NIRUX accepts as a name (not a path).
fn valid_sound(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

pub fn argv(parts: &[&str]) -> Vec<String> {
    std::iter::once("sounds")
        .chain(parts.iter().copied())
        .map(str::to_string)
        .collect()
}

pub fn set_enabled(on: bool) -> Vec<String> {
    argv(&[if on { "on" } else { "off" }])
}

pub fn toggle_event(event: &str) -> Vec<String> {
    argv(&["toggle", event])
}

pub fn test_event(event: &str) -> Vec<String> {
    argv(&["test", event])
}

pub fn set_app(app: &str, sound: &str) -> Vec<String> {
    argv(&["app", app, sound])
}

pub fn test_app(app: &str) -> Vec<String> {
    argv(&["test", "notification", "--app", app])
}

pub fn remove_app(app: &str) -> Vec<String> {
    argv(&["app", app, "remove"])
}

/// Directories whose `*.ogg` names can be chosen: the user's sounds next to
/// `sounds.toml`, then the NIRUX sounds.
pub fn sound_dirs(status: &Status) -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let user = Path::new(&status.config)
        .parent()
        .map(|dir| dir.join("sounds"))
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("nirux/sounds")
        });
    let prefix = std::env::var_os("NIRUX_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local"));
    vec![user, prefix.join("share/nirux/sounds")]
}

/// Sound names from those directories, sorted and without repeats.
pub fn sound_names(dirs: &[PathBuf]) -> Vec<String> {
    let mut names: Vec<String> = dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".ogg")?;
            valid_sound(stem).then(|| stem.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names.truncate(64);
    names
}

/// App names from `nirunotify --history` (id, app, …), newest first, valid
/// and not mapped yet.
pub fn parse_history(text: &str, known: &[App]) -> Vec<String> {
    let mut rows: Vec<(u64, String)> = text
        .lines()
        .enumerate()
        .filter_map(|(line, row)| {
            let mut fields = row.split('\t');
            let id = fields.next()?.trim();
            let app = app_key(fields.next()?);
            let order = id.parse::<u64>().unwrap_or(line as u64);
            valid_app(&app).then_some((order, app))
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let mut seen: Vec<String> = known.iter().map(|app| app.app.clone()).collect();
    let mut out = Vec::new();
    for (_, app) in rows {
        if !seen.contains(&app) {
            seen.push(app.clone());
            out.push(app);
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

pub fn history_apps(known: &[App]) -> Vec<String> {
    Command::new("timeout")
        .args(["-k", "0.2", "1", "nirunotify", "--history"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let bytes = &output.stdout[..output.stdout.len().min(256 * 1024)];
            parse_history(&String::from_utf8_lossy(bytes), known)
        })
        .unwrap_or_default()
}

/// How an app's state reads in the Sound view.
pub fn state_text(app: &App) -> &'static str {
    match app.state.as_str() {
        "ok" => "ok",
        "missing" => "missing · plays the notification sound",
        "silent" => "silent",
        "default" => "default · the notification sound",
        _ => "unknown",
    }
}

/// `thunderbird` → `Thunderbird`.
pub fn display_name(app: &str) -> String {
    let mut chars = app.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"schema":1,"enabled":true,"volume":40,"config":"/home/u/.config/nirux/sounds.toml","events":{"startup":true,"notification":true,"update":false,"shutdown":true},"exclude":["nplay"],"apps":[{"app":"thunderbird","sound":"mail","state":"ok","file":"/home/u/.local/share/nirux/sounds/mail.ogg"},{"app":"signal","sound":"off","state":"silent","file":""},{"app":"discord","sound":"nosuchsound","state":"missing","file":""}]}"#;

    #[test]
    fn parses_schema_one() {
        let status = parse(FIXTURE.as_bytes()).unwrap();
        assert!(status.enabled);
        assert_eq!(status.volume, 40);
        assert!(!status.events.get("update") && status.events.get("startup"));
        assert_eq!(status.apps.len(), 3);
        assert_eq!(status.apps[0].app, "thunderbird");
        assert_eq!(
            state_text(&status.apps[2]),
            "missing · plays the notification sound"
        );
    }

    #[test]
    fn an_older_nirux_is_unavailable() {
        // NIRUX before per-app sounds prints its text status for any extra word.
        let text = b"Sounds: on \xc2\xb7 volume 35% \xc2\xb7 config /x\n  startup: on\n";
        assert!(parse(text).is_err());
        assert!(parse(br#"{"schema":2,"enabled":true}"#).is_err());
    }

    #[test]
    fn every_action_has_its_nirux_argv() {
        assert_eq!(set_enabled(false), ["sounds", "off"]);
        assert_eq!(set_enabled(true), ["sounds", "on"]);
        assert_eq!(toggle_event("update"), ["sounds", "toggle", "update"]);
        assert_eq!(test_event("startup"), ["sounds", "test", "startup"]);
        assert_eq!(
            set_app("signal", "chat"),
            ["sounds", "app", "signal", "chat"]
        );
        assert_eq!(
            set_app("signal", "default"),
            ["sounds", "app", "signal", "default"]
        );
        assert_eq!(set_app("signal", "off"), ["sounds", "app", "signal", "off"]);
        assert_eq!(
            test_app("thunderbird"),
            ["sounds", "test", "notification", "--app", "thunderbird"]
        );
        assert_eq!(
            remove_app("discord"),
            ["sounds", "app", "discord", "remove"]
        );
    }

    #[test]
    fn app_names_follow_the_nirux_rule() {
        for good in ["signal", "Thunderbird", "org.gnome.Nautilus", "my_app-2"] {
            assert!(valid_app(good), "{good}");
        }
        for bad in [
            "",
            "bad name!",
            "a/b",
            "x;rm",
            "$(id)",
            "ä",
            &"a".repeat(65),
        ] {
            assert!(!valid_app(bad), "{bad}");
        }
        assert_eq!(app_key(" Thunderbird.desktop "), "thunderbird");
    }

    #[test]
    fn history_suggests_new_apps_newest_first() {
        let status = parse(FIXTURE.as_bytes()).unwrap();
        let history = "1\tNIRUX\tWelcome\n2\tThunderbird\tMail\n3\tSignal\tHi\n4\tKitty\tDone\n5\tGoogle Chrome\tTab\n6\tnirux\tAgain\n7\tElement\tMsg\n";
        assert_eq!(
            parse_history(history, &status.apps),
            ["element", "nirux", "kitty"],
            "unique, newest first, mapped and invalid names left out"
        );
    }

    #[test]
    fn sound_names_come_from_both_directories() {
        let dir = std::env::temp_dir().join(format!("nl-sounds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (user, nirux) = (dir.join("user"), dir.join("nirux"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&nirux).unwrap();
        for (base, name) in [
            (&nirux, "mail.ogg"),
            (&nirux, "chat.ogg"),
            (&nirux, "notification.ogg"),
            (&user, "mail.ogg"),
            (&user, "ping.ogg"),
            (&user, "bad name.ogg"),
            (&user, "readme.txt"),
        ] {
            std::fs::write(base.join(name), b"OggS").unwrap();
        }
        assert_eq!(
            sound_names(&[user, nirux]),
            ["chat", "mail", "notification", "ping"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
        let status = parse(FIXTURE.as_bytes()).unwrap();
        assert_eq!(
            sound_dirs(&status)[0],
            Path::new("/home/u/.config/nirux/sounds")
        );
    }
}
