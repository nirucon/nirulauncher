//! NIRULANCHER — native application and desktop-action launcher.
//!
//! Keyboard-driven, one process per invocation. No indexing daemon, no network
//! and no shell: applications are started from argv arrays parsed from their
//! desktop entries.

mod config;
mod desktop;
mod render;
mod websearch;

use config::Config;
use render::Renderer;
use std::process::{Command, Stdio};
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
struct Rect {
    x: i16,
    y: i16,
    w: u16,
    h: u16,
}

#[derive(Clone)]
enum Action {
    Logout,
    Reboot,
    Shutdown,
    ResetBar,
}

#[derive(Clone)]
enum Target {
    Spawn {
        argv: Vec<String>,
        terminal: bool,
        path: Option<std::path::PathBuf>,
    },
    Focus(Window),
    Confirm(Action),
    Info,
}

#[derive(Clone)]
struct Item {
    /// Stable key for favourites and local usage.
    key: String,
    group: String,
    name: String,
    detail: String,
    favorite: bool,
    target: Target,
}

impl Item {
    fn label(&self) -> String {
        if self.group.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.group, self.name)
        }
    }
}

struct Ui {
    c: RustConnection,
    root: Window,
    win: Window,
    gc: Gcontext,
    cfg: Config,
    renderer: Renderer,
    items: Vec<Item>,
    filtered: Vec<usize>,
    usage: std::collections::HashMap<String, u64>,
    query: String,
    selected: usize,
    offset: usize,
    rows: usize,
    width: u16,
    height: u16,
    row_height: i16,
    padding: i16,
    confirm: Option<Action>,
    error: String,
    selection: Atom,
    activate: Atom,
    /// True when the window was opened for a web search (`--search`).
    web_mode: bool,
    /// The engines offered for the typed text.
    sources: Vec<websearch::Source>,
    /// How many items existed before the first web-search row was added.
    plain_items: usize,
}

/// Shorter text filters apps; from here on it also searches the web.
const MIN_SEARCH_CHARS: usize = 3;

fn intern(c: &RustConnection, name: &str) -> Result<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn set_utf8(c: &RustConnection, win: Window, atom: Atom, value: &str) -> Result<()> {
    let utf8 = intern(c, "UTF8_STRING")?;
    c.change_property8(PropMode::REPLACE, win, atom, utf8, value.as_bytes())?;
    Ok(())
}

/// Keyboard-accessible input controls. Device IDs and state come from niructl;
/// no microphone or mute setting changes just by opening this view.
fn short_device(name: &str) -> String {
    let tail = name
        .rsplit_once("Audio Controller ")
        .map(|(_, tail)| tail)
        .unwrap_or(name)
        .trim();
    let tail = tail
        .strip_prefix("Smart Sound Technology ")
        .unwrap_or(tail)
        .trim();
    let mut chars = tail.chars();
    let short: String = chars.by_ref().take(44).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

fn microphone_items() -> Vec<Item> {
    let mut items = Vec::new();
    for (key, name, detail, args) in [
        (
            "mute",
            "Mute microphone",
            "Toggle input mute",
            vec!["mic-mute"],
        ),
        (
            "up",
            "Microphone level up",
            "Increase input gain by 5%; keep mute",
            vec!["mic-level", "up"],
        ),
        (
            "down",
            "Microphone level down",
            "Decrease input gain by 5%; keep mute",
            vec!["mic-level", "down"],
        ),
    ] {
        items.push(Item {
            key: format!("action:microphone:{key}"),
            group: "Audio".into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: Target::Spawn {
                argv: ["niructl", "audio"]
                    .into_iter()
                    .chain(args)
                    .map(str::to_string)
                    .collect(),
                terminal: false,
                path: None,
            },
        });
    }
    match Command::new("niructl").args(["audio", "sources"]).output() {
        Ok(output) if output.status.success() => {
            let mut found = false;
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let default = line.starts_with('*');
                let Some((id, name)) = line.trim_start_matches('*').split_once('\t') else {
                    continue;
                };
                if id.parse::<u32>().is_err() || name.is_empty() {
                    continue;
                }
                found = true;
                items.push(Item {
                    key: format!("action:audio:sources:{id}"),
                    group: "Audio".into(),
                    name: format!("Mic: {}", short_device(name)),
                    detail: if default {
                        "Current input"
                    } else {
                        "Use this input"
                    }
                    .into(),
                    favorite: false,
                    target: Target::Spawn {
                        argv: vec!["niructl".into(), "audio".into(), "source".into(), id.into()],
                        terminal: false,
                        path: None,
                    },
                });
            }
            if !found {
                items.push(microphone_info(
                    "No microphone",
                    "No audio input is available",
                ));
            }
        }
        _ => items.push(microphone_info(
            "Microphone unavailable",
            "niructl / WirePlumber did not respond",
        )),
    }
    items
}

fn bluetooth_items() -> Vec<Item> {
    let mut items = Vec::new();
    let Ok(output) = Command::new("niructl")
        .args(["bluetooth", "status"])
        .output()
    else {
        return vec![bluetooth_info(
            "Bluetooth unavailable",
            "niructl did not respond",
        )];
    };
    if !output.status.success() {
        return vec![bluetooth_info(
            "Bluetooth unavailable",
            "BlueZ status failed",
        )];
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut powered = false;
    let mut adapter = false;
    let mut devices = Vec::new();
    for line in text.lines() {
        if line == "powered=true" {
            powered = true;
        }
        if let Some(value) = line.strip_prefix("adapter=") {
            adapter = !value.is_empty();
        }
        if let Some(rest) = line.strip_prefix("device=") {
            let fields: Vec<&str> = rest.split('\t').collect();
            if fields.len() == 4 && !fields[0].is_empty() {
                devices.push((
                    fields[0].to_string(),
                    fields[1].to_string(),
                    fields[2] == "true",
                    fields[3].to_string(),
                ));
            }
        }
    }
    if !adapter {
        return vec![bluetooth_info(
            "No Bluetooth adapter",
            "No Bluetooth hardware was reported",
        )];
    }
    items.push(Item {
        key: "action:bluetooth:power".into(),
        group: "Bluetooth".into(),
        name: if powered {
            "Turn Bluetooth off".into()
        } else {
            "Turn Bluetooth on".into()
        },
        detail: if powered {
            "Currently on"
        } else {
            "Currently off"
        }
        .into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec![
                "niructl".into(),
                "bluetooth".into(),
                "power".into(),
                "toggle".into(),
            ],
            terminal: false,
            path: None,
        },
    });
    if devices.is_empty() {
        items.push(bluetooth_info(
            "No paired devices",
            "Pair devices with your Bluetooth manager first",
        ));
    }
    for (mac, name, connected, battery) in devices {
        items.push(Item {
            key: format!("action:bluetooth:{mac}"),
            group: "Bluetooth".into(),
            name: format!(
                "{}: {name}",
                if connected { "Disconnect" } else { "Connect" }
            ),
            detail: if battery.is_empty() {
                "Paired device".into()
            } else {
                format!("Battery {battery}% · paired")
            },
            favorite: false,
            target: Target::Spawn {
                argv: vec![
                    "niructl".into(),
                    "bluetooth".into(),
                    if connected { "disconnect" } else { "connect" }.into(),
                    mac,
                ],
                terminal: false,
                path: None,
            },
        });
    }
    items
}

fn power_items() -> Vec<Item> {
    let Ok(output) = Command::new("niructl")
        .args(["powerprofile", "status"])
        .output()
    else {
        return vec![Item {
            key: "power:unavailable".into(),
            group: "Power".into(),
            name: "Power profiles unavailable".into(),
            detail: "niructl did not respond".into(),
            favorite: false,
            target: Target::Info,
        }];
    };
    if !output.status.success() {
        return vec![Item {
            key: "power:unavailable".into(),
            group: "Power".into(),
            name: "Power profiles unavailable".into(),
            detail: "power-profiles-daemon is not available".into(),
            favorite: false,
            target: Target::Info,
        }];
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let active = text
        .lines()
        .find_map(|line| line.strip_prefix("active="))
        .unwrap_or("unknown");
    let source = text
        .lines()
        .find_map(|line| line.strip_prefix("power="))
        .unwrap_or("unknown");
    let mut items = Vec::new();
    for line in text.lines() {
        let Some(profile) = line.strip_prefix("profile=") else {
            continue;
        };
        if profile.is_empty() {
            continue;
        }
        items.push(Item {
            key: format!("power:{profile}"),
            group: "Power".into(),
            name: format!("Power profile: {profile}"),
            detail: if profile == active {
                format!("Current · power source {source}")
            } else {
                format!("Use this profile · source {source}")
            },
            favorite: false,
            target: spawn_niructl(&["powerprofile", "set", profile]),
        });
    }
    if items.is_empty() {
        items.push(Item {
            key: "power:none".into(),
            group: "Power".into(),
            name: "No power profiles".into(),
            detail: "power-profiles-daemon reported no profiles".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    items
}

/// Find an installed program the same way the shell would.
///
/// `~/.local/bin` is on the session path, but the launcher may be started
/// before that is set, so it is checked explicitly.
fn in_path(name: &str) -> bool {
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".local/share/nirux/current/bin"));
    }
    dirs.iter().any(|dir| dir.join(name).is_file())
}

/// Read the newer NIRUX version the bar cached, if any.
///
/// The bar checks GitHub once a day and writes `~/.cache/nirubar/nirux-update`.
/// Reading that file keeps the launcher and the bar in agreement and avoids a
/// second network request. Two version strings are compared so a stale cache
/// from an older install is ignored.
fn nirux_update_available() -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"))
        .join("nirubar/nirux-update");
    let latest = std::fs::read_to_string(cache).ok()?.trim().to_string();
    let installed = installed_nirux_version()?;
    is_newer_version(&latest, &installed).then_some(latest)
}

/// The active generation directory ends in the version.
fn installed_nirux_version() -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let target = std::fs::read_link(home.join(".local/share/nirux/current")).ok()?;
    let name = target.file_name()?.to_string_lossy().to_string();
    let version = name.split('-').next()?.to_string();
    (version.split('.').count() >= 2
        && version
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
    .then_some(version)
}

/// Compare dotted versions numerically, treating a missing part as zero.
fn is_newer_version(candidate: &str, installed: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(installed));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

fn spawn_niructl(args: &[&str]) -> Target {
    Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|arg| (*arg).to_string()))
            .collect(),
        terminal: false,
        path: None,
    }
}

fn display_items() -> Vec<Item> {
    let mut items = Vec::new();
    if let Ok(output) = Command::new("niructl").args(["display", "list"]).output() {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                if fields.len() < 2 || fields[0].is_empty() {
                    continue;
                }
                items.push(Item {
                    key: format!("display:{}", fields[0]),
                    group: "Displays".into(),
                    name: format!("Display: {}", fields[0]),
                    detail: format!("{} output(s) · preview with revert", fields[1]),
                    favorite: false,
                    target: spawn_niructl(&["display", "preview", fields[0]]),
                });
            }
        }
    }
    // The layout as it is now becomes a profile in one step; rotating a single
    // screen is the common fix for a display mounted upside down.
    for (key, name, detail, command) in [(
        "display:auto",
        "Displays: Arrange side by side",
        "Preferred mode on every connected screen",
        vec!["display", "auto"],
    )] {
        items.push(Item {
            key: key.into(),
            group: "Displays".into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: spawn_niructl(&command),
        });
    }
    if let Ok(output) = Command::new("niructl").args(["monitors"]).output() {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let fields: Vec<&str> = line.split_whitespace().collect();
                // `  DisplayPort-5 2560x1440+2560+0` is one screen; the
                // trailing note lines never start with an indented output name.
                if fields.len() != 2 || !fields[1].contains('x') || !fields[1].contains('+') {
                    continue;
                }
                let output_name = fields[0];
                for rotation in ["normal", "left", "right", "inverted"] {
                    items.push(Item {
                        key: format!("display:rotate:{output_name}:{rotation}"),
                        group: "Displays".into(),
                        name: format!("Displays: Turn {output_name} {rotation}"),
                        detail: format!("{} · keeps every other screen", fields[1]),
                        favorite: false,
                        target: spawn_niructl(&["display", "rotate", output_name, rotation]),
                    });
                }
            }
        }
    }
    if let Ok(output) = Command::new("niructl").args(["display", "status"]).output() {
        let status = String::from_utf8_lossy(&output.stdout);
        if status.trim_start().starts_with("display preview ") && !status.contains("off") {
            items.push(Item {
                key: "display:keep".into(),
                group: "Displays".into(),
                name: "Keep display layout".into(),
                detail: "Confirm the preview".into(),
                favorite: false,
                target: spawn_niructl(&["display", "keep"]),
            });
            items.push(Item {
                key: "display:revert".into(),
                group: "Displays".into(),
                name: "Revert display layout".into(),
                detail: "Return to the previous layout".into(),
                favorite: false,
                target: spawn_niructl(&["display", "revert"]),
            });
        }
    }
    if items.is_empty() {
        items.push(Item {
            key: "display:none".into(),
            group: "Displays".into(),
            name: "No display profiles".into(),
            detail: "Create one with niructl display save NAME".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    items
}

fn bluetooth_info(name: &str, detail: &str) -> Item {
    Item {
        key: "bluetooth:status".into(),
        group: "Bluetooth".into(),
        name: name.into(),
        detail: detail.into(),
        favorite: false,
        target: Target::Info,
    }
}

fn microphone_info(name: &str, detail: &str) -> Item {
    Item {
        key: "microphone:status".into(),
        group: "Audio".into(),
        name: name.into(),
        detail: detail.into(),
        favorite: false,
        target: Target::Info,
    }
}

fn shortcut_key_label(raw: &str) -> String {
    raw.split('+')
        .map(|part| match part {
            "super" => "Super",
            "ctrl" => "Ctrl",
            "shift" => "Shift",
            "alt" => "Alt",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn shortcut_action_label(action: &str) -> String {
    let name = match action {
        "terminal" => "Terminal",
        "terminal_alt" => "Alternative terminal",
        "browser" => "Browser",
        "launcher" => "Application launcher",
        "launcher_unified" => "Actions and layouts",
        "window_switcher" => "Window switcher",
        "lock" => "Lock screen",
        "reload" => "Reload",
        "nightlight_toggle" => "Night light",
        "dnd_toggle" => "Toggle DND",
        "idle_toggle" => "Toggle idle inhibition",
        "save_session" => "Save session",
        "toggle_gaps" => "Toggle gaps",
        "close" => "Close window",
        "quit" => "Log out",
        "toggle_float" => "Toggle floating",
        "toggle_fullscreen" => "Fullscreen",
        "scratchpad" => "Scratchpad",
        "focus_next" => "Next window",
        "focus_prev" => "Previous window",
        "move_next" => "Move window next",
        "move_prev" => "Move window previous",
        "focusmon -1" => "Previous monitor",
        "focusmon 1" => "Next monitor",
        "tagmon -1" => "Move to previous monitor",
        "tagmon 1" => "Move to next monitor",
        "tag_next" => "Next workspace",
        "tag_prev" => "Previous workspace",
        "layout_next" => "Next layout",
        "layout_prev" => "Previous layout",
        "shell nirushot --full" => "Screenshot: full screen",
        "shell nirushot --region" => "Screenshot: region",
        "shell nirushot --window" => "Screenshot: window",
        _ => {
            if let Some(rest) = action.strip_prefix("media_key ") {
                return format!("Media key: {rest}");
            }
            if let Some(rest) = action.strip_prefix("brightness_step ") {
                return format!("Brightness: {rest}");
            }
            if let Some(rest) = action.strip_prefix("layout ") {
                return format!("Layout: {rest}");
            }
            if let Some(rest) = action.strip_prefix("tag ") {
                return format!("Workspace: {rest}");
            }
            if let Some(rest) = action.strip_prefix("movetotag ") {
                return format!("Move to workspace: {rest}");
            }
            if let Some(rest) = action.strip_prefix("toggletag ") {
                return format!("Toggle workspace: {rest}");
            }
            if let Some(rest) = action.strip_prefix("togglemovetotag ") {
                return format!("Move and toggle workspace: {rest}");
            }
            if let Some(rest) = action.strip_prefix("shell ") {
                return format!("Custom: {rest}");
            }
            return action.replace('_', " ");
        }
    };
    name.into()
}

fn action_items(cfg: &Config) -> Vec<Item> {
    let mut items = Vec::new();
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };
    for (name, detail, action) in [
        ("Lock screen", "Lock now", "lock"),
        ("Screensaver", "Show the screensaver now", "screensaver"),
        (
            "Reload configuration",
            "Apply config.toml and theme",
            "reload",
        ),
        ("Toggle notifications", "Pause or resume DND", "dnd toggle"),
        ("Toggle keep-awake", "Inhibit idle lock/DPMS", "idle toggle"),
        (
            "Brightness up",
            "Increase the backlight by 5%",
            "brightness up",
        ),
        (
            "Brightness down",
            "Decrease the backlight by 5%",
            "brightness down",
        ),
        (
            "Night light",
            "Toggle the manual night light",
            "nightlight toggle",
        ),
        (
            "Compositor",
            "Toggle picom transparency and shadows",
            "compositor toggle",
        ),
        ("Layout: Tile", "Super+T", "layout tile"),
        ("Layout: Left stack", "Super+Y", "layout left"),
        ("Layout: Monocle", "Super+M", "layout monocle"),
        ("Layout: Centered", "Super+C", "layout centered"),
        (
            "Layout: Centered floating",
            "Super+U",
            "layout centeredfloat",
        ),
        ("Layout: Floating", "Super+F", "layout floating"),
        ("Layout: Fibonacci", "Super+Ctrl+E", "layout fibonacci"),
        ("Previous layout", "Super+Ctrl+,", "layout_prev"),
        ("Next layout", "Super+Ctrl+.", "layout_next"),
        ("Suspend", "Lock, then suspend", "suspend"),
        ("Mute audio", "Toggle the default output", "audio mute"),
    ] {
        let mut parts = action.split(' ');
        let first = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();
        let mut argv = vec!["niructl".to_string(), first.to_string()];
        argv.extend(rest.into_iter().map(str::to_string));
        let group = if matches!(first, "lock" | "screensaver" | "reload" | "dnd" | "idle") {
            "System"
        } else if first == "audio" {
            "Audio"
        } else if first == "compositor" {
            "Appearance"
        } else if first == "layout" {
            "Layouts"
        } else if first == "mode" {
            "Work modes"
        } else {
            "Display"
        };
        items.push(Item {
            key: format!("action:{name}"),
            group: group.into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: Target::Spawn {
                argv,
                terminal: false,
                path: None,
            },
        });
    }
    for theme in [
        "niru-noir",
        "niru-nord",
        "niru-satie",
        "niru-carl-larsson",
        "niru-forest-vigil",
        "niru-blood-of-urdr",
        "niru-root-and-stone",
        "niru-blue-horizon",
        "niru-the-norns",
        "niru-winter-nights",
    ] {
        items.push(Item {
            key: format!("action:theme:{theme}"),
            group: "Appearance".into(),
            name: format!("Theme: {theme}"),
            detail: "Apply across WM, bar, saver and lock".into(),
            favorite: false,
            target: niructl(&["theme", theme]),
        });
    }
    // Choosing an image, the scale, rotation and theme following belong to
    // NIRU Wallpaper, which shows previews and the per-screen choice. The
    // launcher keeps the one entry that opens it instead of repeating a list
    // of files here; themes stay in this launcher, under Appearance.

    // Audio output/source selection uses niructl -> wpctl. The ID, not the
    // display name, is authoritative because two devices may share a name.
    for (kind, label) in [("outputs", "Audio output")] {
        if let Ok(out) = Command::new("niructl").args(["audio", kind]).output() {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let (default, rest) = match line.strip_prefix('*') {
                        Some(rest) => (true, rest),
                        None => (false, line),
                    };
                    let Some((id, name)) = rest.split_once('\t') else {
                        continue;
                    };
                    if id.is_empty() || name.is_empty() {
                        continue;
                    }
                    items.push(Item {
                        key: format!("action:audio:{kind}:{id}"),
                        group: "Audio".into(),
                        name: format!("{label}: {}", short_device(name)),
                        detail: if default {
                            "Current device".into()
                        } else {
                            "Use this device".into()
                        },
                        favorite: false,
                        target: niructl(&["audio", kind.trim_end_matches('s'), id]),
                    });
                }
            }
        }
    }

    // Effective shortcut catalogue from NIRUWM. This includes user overrides
    // and skips explicit unbinds; the launcher never invents a second truth.
    let mut shortcuts: Vec<(String, Vec<String>)> = Vec::new();
    if let Ok(out) = Command::new("niructl").arg("keys").output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let Some((key, action)) = line.split_once('\t') else {
                    continue;
                };
                if key.is_empty() || action.is_empty() || action == "unbind" {
                    continue;
                }
                let key = shortcut_key_label(key);
                if let Some((_, keys)) = shortcuts.iter_mut().find(|(a, _)| a == action) {
                    keys.push(key);
                } else {
                    shortcuts.push((action.to_string(), vec![key]));
                }
            }
        }
    }
    for (action, keys) in shortcuts {
        items.push(Item {
            key: format!("shortcut:{action}"),
            group: "Shortcuts".into(),
            name: shortcut_action_label(&action),
            detail: keys.join(" / "),
            favorite: false,
            target: Target::Info,
        });
    }
    items.push(Item {
        key: "shortcut:mouse".into(),
        group: "Shortcuts".into(),
        name: "Mouse move / resize".into(),
        detail: "Super+left / right drag".into(),
        favorite: false,
        target: Target::Info,
    });
    items.extend(microphone_items());
    items.extend(bluetooth_items());
    items.extend(display_items());
    items.extend(power_items());

    if let Ok(out) = Command::new("niructl")
        .args(["explain", "--launcher"])
        .output()
    {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let mut fields = line.splitn(4, '\t');
                let level = fields.next().unwrap_or("");
                let name = fields.next().unwrap_or("");
                let detail = fields.next().unwrap_or("");
                let action = fields.next().unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let target = match action {
                    "doctor" => Target::Spawn {
                        argv: vec!["nirux".into(), "doctor".into()],
                        terminal: true,
                        path: None,
                    },
                    "audio" => Target::Spawn {
                        argv: vec!["niructl".into(), "audio".into(), "outputs".into()],
                        terminal: true,
                        path: None,
                    },
                    "dnd" => niructl(&["dnd", "toggle"]),
                    "reload" => niructl(&["reload"]),
                    _ => Target::Info,
                };
                items.push(Item {
                    key: format!("diagnostic:{name}"),
                    group: "Diagnostics".into(),
                    name: name.into(),
                    detail: if level == "WARN" {
                        format!("Needs attention: {detail}")
                    } else {
                        detail.into()
                    },
                    favorite: false,
                    target,
                });
            }
        }
    }
    // A newer NIRUX release, read from the same cache the bar uses. The action
    // runs the update in a terminal: it needs sudo and switches the program
    // generation, so it must not happen behind the user's back.
    if let Some(latest) = nirux_update_available() {
        items.push(Item {
            key: "action:nirux-update".into(),
            group: "System".into(),
            name: format!("Update NIRUX to {latest}"),
            detail: "Runs in a terminal · log out afterwards".into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["nirux".into(), "update".into()],
                terminal: true,
                path: None,
            },
        });
    }
    items.push(Item {
        key: "action:diagnostics".into(),
        group: "System".into(),
        name: "Run diagnostics".into(),
        detail: "Open the full NIRUX health report".into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["nirux".into(), "doctor".into()],
            terminal: true,
            path: None,
        },
    });

    if let Ok(out) = Command::new("niructl").args(["recipe", "list"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let mut fields = line.splitn(3, '\t');
                let name = fields.next().unwrap_or("");
                let count = fields.next().unwrap_or("");
                let detail = fields.next().unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                items.push(Item {
                    key: format!("recipe:{name}"),
                    group: "Recipes".into(),
                    name: name.into(),
                    detail: if detail.is_empty() {
                        format!("{count} apps")
                    } else {
                        format!("{count} apps · {detail}")
                    },
                    favorite: false,
                    target: niructl(&["recipe", "run", name]),
                });
            }
        }
    }

    // One wallpaper entry, offering the window that owns these settings.
    // It is shown only when it is installed, so a system that predates it
    // never shows a dead entry. The name carries the words a user types,
    // because the list shows about a dozen rows and matches the name before
    // the detail.
    if in_path("niruwallpaper") {
        items.push(Item {
            key: "action:wallpaper:open".into(),
            group: "Wallpaper".into(),
            name: "Wallpaper background and appearance: open settings".into(),
            detail: "NIRU Wallpaper: choose an image, scale, rotation and the screen".into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["niruwallpaper".to_string()],
                terminal: false,
                path: None,
            },
        });
    }

    for ws in 1..=10 {
        items.push(Item {
            key: format!("action:workspace:{ws}"),
            group: "Workspaces".into(),
            name: format!("Workspace {ws}"),
            detail: "Switch the focused monitor".into(),
            favorite: false,
            target: niructl(&["tag", &ws.to_string()]),
        });
    }
    items.push(Item {
        key: "action:nirux-update".into(),
        group: "System".into(),
        name: "Update NIRUX suite…".into(),
        detail: "Run nirux update in a terminal".into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["nirux".into(), "update".into()],
            terminal: true,
            path: None,
        },
    });
    items.push(Item {
        key: "action:reset-bar".into(),
        group: "Appearance".into(),
        name: "Reset bar configuration…".into(),
        detail: "Restore current NIRUBAR defaults; a backup is kept".into(),
        favorite: false,
        target: Target::Confirm(Action::ResetBar),
    });
    for (name, detail, action) in [
        ("Log out…", "Requires confirmation", Action::Logout),
        ("Reboot…", "Requires confirmation", Action::Reboot),
        ("Shut down…", "Requires confirmation", Action::Shutdown),
    ] {
        items.push(Item {
            key: format!("action:{name}"),
            group: "Power".into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: Target::Confirm(action),
        });
    }
    let _ = cfg;
    items
}

fn app_items(apps: Vec<desktop::App>) -> Vec<Item> {
    apps.into_iter()
        .map(|app| Item {
            key: app.id.clone(),
            group: "Applications".into(),
            name: app.name,
            detail: app.id,
            favorite: false,
            target: Target::Spawn {
                argv: app.argv,
                terminal: app.terminal,
                path: app.path,
            },
        })
        .collect()
}

/// Open top-level windows from EWMH, for the action launcher's Windows group.
fn window_items(c: &RustConnection, root: Window) -> Vec<Item> {
    let Ok(atom) = intern(c, "_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    let Ok(reply) = c.get_property(false, root, atom, AtomEnum::ANY, 0, 128) else {
        return Vec::new();
    };
    let Ok(reply) = reply.reply() else {
        return Vec::new();
    };
    let windows: Vec<Window> = reply
        .value32()
        .map(|values| values.collect())
        .unwrap_or_default();
    let name_atom = intern(c, "_NET_WM_NAME").ok();
    let utf8 = intern(c, "UTF8_STRING").ok();
    let mut items = Vec::new();
    for window in windows {
        let class = c
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()
            .and_then(|r| r.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .unwrap_or_default();
        if class.contains("NIRULANCHER") || class.contains("NIRULOCK") {
            continue;
        }
        let title = name_atom
            .zip(utf8)
            .and_then(|(atom, utf8)| {
                c.get_property(false, window, atom, utf8, 0, 1024)
                    .ok()?
                    .reply()
                    .ok()
            })
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                c.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)
                    .ok()?
                    .reply()
                    .ok()
                    .map(|r| String::from_utf8_lossy(&r.value).to_string())
            })
            .unwrap_or_else(|| format!("Window {window}"));
        items.push(Item {
            key: format!("window:{window}"),
            group: "Windows".into(),
            name: title,
            detail: "Switch to this window".into(),
            favorite: false,
            target: Target::Focus(window),
        });
    }
    items.truncate(64);
    items
}

fn usage_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/state")
        });
    base.join("nirulauncher/usage.tsv")
}

fn load_usage() -> std::collections::HashMap<String, u64> {
    let mut usage = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(usage_path()) else {
        return usage;
    };
    for line in text.lines().take(500) {
        let Some((key, stamp)) = line.rsplit_once('\t') else {
            continue;
        };
        if let Ok(stamp) = stamp.parse::<u64>() {
            usage.insert(key.to_string(), stamp);
        }
    }
    usage
}

fn record_usage(key: &str) {
    let mut usage = load_usage();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    usage.insert(key.to_string(), now);
    let mut rows: Vec<(&String, &u64)> = usage.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    rows.truncate(200);
    let text: String = rows
        .into_iter()
        .map(|(key, stamp)| format!("{key}\t{stamp}\n"))
        .collect();
    if let Some(parent) = usage_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(usage_path(), text);
}

fn favorites_path() -> std::path::PathBuf {
    usage_path().with_file_name("favorites.tsv")
}

fn removed_favorites_path() -> std::path::PathBuf {
    usage_path().with_file_name("favorites-removed.tsv")
}

fn load_removed_favorites() -> Vec<String> {
    std::fs::read_to_string(removed_favorites_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn save_removed_favorites(keys: &[String]) {
    let path = removed_favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text: String = keys.iter().map(|key| format!("{key}\n")).collect();
    let _ = std::fs::write(path, text);
}

fn load_runtime_favorites() -> Vec<String> {
    std::fs::read_to_string(favorites_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(5)
        .map(str::to_string)
        .collect()
}

fn save_runtime_favorites(keys: &[String]) {
    let path = favorites_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text: String = keys.iter().take(5).map(|key| format!("{key}\n")).collect();
    let _ = std::fs::write(path, text);
}

fn filtered(
    items: &[Item],
    query: &str,
    _usage: &std::collections::HashMap<String, u64>,
) -> Vec<usize> {
    let mut favorites: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.favorite)
        .map(|(index, _)| index)
        .collect();
    favorites.sort_by_key(|index| items[*index].label().to_lowercase());
    favorites.truncate(5);
    let pinned: std::collections::HashSet<usize> = favorites.into_iter().collect();
    let q = query.trim().to_lowercase();
    let mut hits: Vec<(bool, u8, String, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let label = item.label().to_lowercase();
            let name = item.name.to_lowercase();
            let detail = item.detail.to_lowercase();
            let rank = if q.is_empty() || name == q || label == q {
                0
            } else if name.starts_with(&q) || label.starts_with(&q) {
                1
            } else if name.contains(&q) || label.contains(&q) {
                2
            } else if detail.contains(&q) {
                3
            } else {
                return None;
            };
            Some((!pinned.contains(&i), rank, label.clone(), i))
        })
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    hits.into_iter().map(|(_, _, _, i)| i).collect()
}

fn monitor_at(c: &RustConnection, root: Window, px: i16, py: i16) -> Rect {
    let fallback = Rect {
        x: 0,
        y: 0,
        w: c.setup().roots[c.setup().roots.len() - 1].width_in_pixels,
        h: c.setup().roots[c.setup().roots.len() - 1].height_in_pixels,
    };
    let Ok(resources) = c.randr_get_screen_resources_current(root) else {
        return fallback;
    };
    let Ok(resources) = resources.reply() else {
        return fallback;
    };
    for output in resources.outputs {
        let Ok(info) = c.randr_get_output_info(output, resources.config_timestamp) else {
            continue;
        };
        let Ok(info) = info.reply() else { continue };
        if info.connection != x11rb::protocol::randr::Connection::CONNECTED || info.crtc == 0 {
            continue;
        }
        let Ok(crtc) = c.randr_get_crtc_info(info.crtc, resources.config_timestamp) else {
            continue;
        };
        let Ok(crtc) = crtc.reply() else { continue };
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }
        let r = Rect {
            x: crtc.x,
            y: crtc.y,
            w: crtc.width,
            h: crtc.height,
        };
        if px >= r.x && px < r.x + r.w as i16 && py >= r.y && py < r.y + r.h as i16 {
            return r;
        }
    }
    fallback
}

fn point(c: &RustConnection, root: Window) -> (i16, i16) {
    c.query_pointer(root)
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| (r.root_x, r.root_y))
        .unwrap_or((0, 0))
}

fn wait_until_viewable(c: &RustConnection, win: Window) -> Result<()> {
    c.flush()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if c.get_window_attributes(win)?.reply()?.map_state == MapState::VIEWABLE {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("window manager did not map the launcher; cannot acquire focus".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

impl Ui {
    fn new(mode: &str) -> Result<Self> {
        let cfg = Config::load()?;
        let apps = desktop::discover();
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        let mut items = Vec::new();
        if mode == "actions" {
            items.extend(action_items(&cfg));
            items.extend(window_items(&c, root));
            items.extend(app_items(apps));
        } else if mode == "microphone" {
            items.extend(microphone_items());
        } else if mode == "bluetooth" {
            items.extend(bluetooth_items());
        } else if mode == "displays" {
            items.extend(display_items());
        } else if mode == "power" {
            items.extend(power_items());
        } else if mode == "windows" {
            items.extend(window_items(&c, root));
        } else if mode == "search" {
            // The address bar: typing opens the text in a search engine.
            items.extend(app_items(apps));
        } else {
            items.extend(app_items(apps));
        }
        let runtime_favorites = load_runtime_favorites();
        let removed_favorites = load_removed_favorites();
        for item in &mut items {
            let configured = cfg.favorites.iter().any(|favorite| {
                favorite == &item.key
                    || favorite.eq_ignore_ascii_case(&item.name)
                    || favorite == &item.label()
            });
            item.favorite = (configured || runtime_favorites.iter().any(|key| key == &item.key))
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
        if items.is_empty() {
            return Err("no launchable applications or actions found".into());
        }
        let (px, py) = point(&c, root);
        let monitor = monitor_at(&c, root, px, py);
        // The web-search row is added when text is typed, so the window is
        // opened tall enough for it from the start instead of resizing later.
        let web_mode = mode == "search";
        let sources = websearch::sources(&cfg.search_sources);
        // One row per engine plus the local matches the window already has.
        let search_slot = if web_mode { sources.len() } else { 0 };
        let rows = (items.len() + search_slot).min(cfg.max_rows).max(1);
        let (_, line_height) = Renderer::measure(cfg.font_path.as_deref(), cfg.font_size)?;
        let row_height = cfg.row_height.max(line_height + 8);
        let padding = cfg.padding.max((line_height / 2).max(8));
        let header = row_height + padding * 2;
        let footer = row_height;
        let width = cfg.width.min((monitor.w as i16).max(200) as u16);
        let height = (header + footer + rows as i16 * row_height)
            .min(monitor.h as i16)
            .max(header + footer + row_height);
        let x = (monitor.x + (monitor.w as i16 - width as i16) / 2).max(monitor.x);
        let y = (monitor.y + (monitor.h as i16 - height) / 2).max(monitor.y);
        let win = c.generate_id()?;
        let gc = c.generate_id()?;
        let renderer = Renderer::new(
            &c,
            root,
            cfg.font_path.as_deref(),
            cfg.font_size,
            width,
            height as u16,
        )?;
        c.create_window(
            0,
            win,
            root,
            x,
            y,
            width,
            height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(cfg.palette.background)
                .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
        )?;
        c.create_gc(gc, win, &CreateGCAux::new())?;
        let utf8 = intern(&c, "UTF8_STRING")?;
        set_utf8(&c, win, intern(&c, "_NET_WM_NAME")?, "NIRULANCHER")?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            utf8,
            b"NIRULANCHER",
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirulauncher\0NIRULANCHER\0",
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            intern(&c, "_NET_WM_WINDOW_TYPE")?,
            AtomEnum::ATOM,
            &[intern(&c, "_NET_WM_WINDOW_TYPE_DIALOG")?],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            intern(&c, "_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            intern(&c, "WM_PROTOCOLS")?,
            AtomEnum::ATOM,
            &[intern(&c, "WM_DELETE_WINDOW")?],
        )?;
        let selection = intern(&c, "_NIRULANCHER_SELECTION")?;
        let activate = intern(&c, "_NIRULANCHER_ACTIVATE")?;
        let owner = c.get_selection_owner(selection)?.reply()?.owner;
        if owner != x11rb::NONE {
            let _ = c.send_event(
                false,
                owner,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, owner, activate, [1, 0, 0, 0, 0]),
            );
            c.flush()?;
            return Err("launcher is already open".into());
        }
        c.set_selection_owner(win, selection, x11rb::CURRENT_TIME)?;
        c.map_window(win)?;
        // A managed MapWindow becomes a MapRequest to the WM, not an
        // immediate map. Grabbing/focusing before its MapNotify causes
        // GrabNotViewable/BadMatch (especially visible in a slower VM).
        wait_until_viewable(&c, win)?;
        // Input isolation while open, but never an override-redirect window:
        // the session lock remains above and can still take over.
        let _ = c.grab_keyboard(
            false,
            win,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
        let _ = c.grab_pointer(
            true,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        );
        c.set_input_focus(InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME)?;
        c.flush()?;
        let usage = load_usage();
        let filtered = filtered(&items, "", &usage);
        let mut ui = Self {
            c,
            root,
            win,
            gc,
            cfg,
            renderer,
            plain_items: items.len(),
            items,
            filtered,
            usage,
            query: String::new(),
            selected: 0,
            offset: 0,
            rows,
            width,
            height: height as u16,
            row_height,
            padding,
            confirm: None,
            error: String::new(),
            selection,
            activate,
            web_mode,
            sources,
        };
        ui.draw()?;
        Ok(ui)
    }

    fn draw(&mut self) -> Result<()> {
        let pal = self.cfg.palette;
        let pad = self.padding;
        let row = self.row_height;
        let header_h = row + pad * 2;
        self.renderer.clear(&self.c, self.gc, pal.background)?;
        self.renderer.fill(
            &self.c,
            self.gc,
            Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: header_h as u16,
            },
            pal.surface,
        )?;
        let title = if self.confirm.is_some() {
            "Confirm"
        } else if self.web_mode {
            "Search the web"
        } else if self.items.iter().all(|item| item.group == "Audio") {
            "Microphone controls"
        } else if self
            .items
            .iter()
            .any(|i| matches!(i.target, Target::Confirm(_)))
        {
            "Search apps and actions"
        } else {
            "Search applications"
        };
        let query = if self.query.is_empty() {
            title.to_string()
        } else {
            format!("{title}: {}", self.query)
        };
        let colour = if self.query.is_empty() {
            pal.muted
        } else {
            pal.foreground
        };
        self.renderer.line(
            &self.c,
            self.gc,
            pad,
            pad + self.renderer.line_height + 2,
            &query,
            colour,
            pal.surface,
        )?;
        let visible = self
            .rows
            .min(self.filtered.len().saturating_sub(self.offset));
        for line in 0..visible {
            let index = self.filtered[self.offset + line];
            let item = &self.items[index];
            let y = header_h + line as i16 * row;
            let selected = self.offset + line == self.selected;
            let bg = if selected {
                pal.selected
            } else {
                pal.background
            };
            if selected {
                self.renderer.fill(
                    &self.c,
                    self.gc,
                    Rectangle {
                        x: pad / 2,
                        y,
                        width: self.width.saturating_sub(pad as u16),
                        height: row as u16,
                    },
                    bg,
                )?;
                self.renderer.fill(
                    &self.c,
                    self.gc,
                    Rectangle {
                        x: pad / 2,
                        y,
                        width: 3,
                        height: row as u16,
                    },
                    pal.accent,
                )?;
            }
            let name_colour = pal.foreground;
            let detail_colour = pal.muted;
            let baseline = y + row / 2 + self.renderer.line_height / 2 - 2;
            let label = if item.favorite {
                format!("★ {}", item.label())
            } else {
                item.label()
            };
            let usable = ((self.width as i16 - pad * 2) / self.renderer.advance.max(1)).max(1);
            let name_width = (usable * 3 / 5).max(1);
            let detail_width = usable.saturating_sub(name_width + 1).max(1);
            let name = truncate(&label, name_width);
            self.renderer
                .line(&self.c, self.gc, pad, baseline, &name, name_colour, bg)?;
            let detail = truncate(&item.detail, detail_width);
            let dx =
                (self.width as i16 - pad - detail.chars().count() as i16 * self.renderer.advance)
                    .max(pad);
            self.renderer
                .line(&self.c, self.gc, dx, baseline, &detail, detail_colour, bg)?;
        }
        let footer = if let Some(action) = &self.confirm {
            format!(
                "Press Enter again to {} · Esc cancels",
                match action {
                    Action::Logout => "log out",
                    Action::Reboot => "reboot",
                    Action::Shutdown => "shut down",
                    Action::ResetBar => "reset the bar configuration",
                }
            )
        } else if !self.error.is_empty() {
            self.error.clone()
        } else {
            "Enter launch · Esc close · ↑↓ move · Ctrl+P favorite".to_string()
        };
        let footer_colour = if self.confirm.is_some() {
            pal.warning
        } else if !self.error.is_empty() {
            pal.error
        } else {
            pal.muted
        };
        let fy = self.height as i16 - self.padding / 2 - 2;
        let footer_width = ((self.width as i16 - pad * 2) / self.renderer.advance.max(1)).max(1);
        let footer = truncate(&footer, footer_width);
        self.renderer.line(
            &self.c,
            self.gc,
            pad,
            fy,
            &footer,
            footer_colour,
            pal.background,
        )?;
        self.renderer
            .present(&self.c, self.gc, self.win, self.width)?;
        self.c.flush()?;
        Ok(())
    }

    fn toggle_favorite(&mut self) -> Result<()> {
        let Some(&index) = self.filtered.get(self.selected) else {
            return Ok(());
        };
        let key = self.items[index].key.clone();
        let mut keys = load_runtime_favorites();
        let mut removed = load_removed_favorites();
        if self.items[index].favorite {
            self.items[index].favorite = false;
            keys.retain(|candidate| candidate != &key);
            if !removed.iter().any(|candidate| candidate == &key) {
                removed.push(key);
            }
            save_runtime_favorites(&keys);
            save_removed_favorites(&removed);
            self.error.clear();
        } else {
            let count = self.items.iter().filter(|item| item.favorite).count();
            if count >= 5 {
                self.error = "Favorites are limited to 5".into();
                self.draw()?;
                return Ok(());
            }
            self.items[index].favorite = true;
            removed.retain(|candidate| candidate != &key);
            if !keys.iter().any(|candidate| candidate == &key) {
                keys.push(key);
            }
            save_runtime_favorites(&keys);
            save_removed_favorites(&removed);
            self.error.clear();
        }
        self.refilter();
        self.draw()?;
        Ok(())
    }

    fn refilter(&mut self) {
        // Drop the rows the previous keystroke built for the web search. They
        // live after every real item, so removing the tail leaves app, action
        // and favourite indices unchanged.
        self.items.truncate(self.plain_items);
        self.filtered = filtered(&self.items, &self.query, &self.usage);
        self.error.clear();
        // A web search belongs to a phrase, not to a short filter. From three
        // characters on the engine rows join the list at the top, so Enter
        // opens the first engine and the local matches stay one Tab away.
        if self.web_mode && self.query.trim().chars().count() >= MIN_SEARCH_CHARS {
            let mut rows = Vec::new();
            for source in &self.sources {
                if let Some(item) = websearch::item(source, &self.query) {
                    rows.push(item);
                }
            }
            if rows.is_empty() {
                self.error = "No search engine is configured; see the launcher README".into();
            } else {
                let start = self.items.len();
                self.items.extend(rows);
                self.filtered.splice(0..0, start..self.items.len());
                self.error = "Enter opens the selected engine · Tab moves · Esc closes".to_string();
            }
        } else if self.web_mode && !self.query.trim().is_empty() {
            self.error = format!("Type at least {MIN_SEARCH_CHARS} characters to search the web");
        }
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
        self.confirm = None;
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let next = (self.selected as i32 + delta).clamp(0, self.filtered.len() as i32 - 1) as usize;
        self.selected = next;
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if self.selected >= self.offset + self.rows {
            self.offset = self.selected + 1 - self.rows;
        }
        self.confirm = None;
    }

    fn execute(&mut self, key: &str, target: Target) -> Result<bool> {
        match target {
            Target::Confirm(action) => {
                if self.confirm.is_some() {
                    let commands: Vec<Vec<String>> = match action {
                        Action::Logout => vec![vec!["niructl".into(), "logout".into()]],
                        Action::Reboot => vec![vec!["niructl".into(), "reboot".into()]],
                        Action::Shutdown => vec![vec!["niructl".into(), "shutdown".into()]],
                        Action::ResetBar => vec![
                            vec!["nirubar".into(), "--reset-config".into()],
                            vec!["niructl".into(), "restart-bar".into()],
                        ],
                    };
                    for command in commands {
                        if let Err(e) = spawn_command(&command, None, false, &self.cfg) {
                            self.error = e;
                            self.confirm = None;
                            self.draw()?;
                            return Ok(true);
                        }
                    }
                    record_usage(key);
                    self.usage = load_usage();
                    return Ok(false);
                }
                self.confirm = Some(action);
                self.draw()?;
                Ok(true)
            }
            Target::Info => Ok(true),
            Target::Focus(window) => {
                let atom = intern(&self.c, "_NET_ACTIVE_WINDOW")?;
                let event =
                    ClientMessageEvent::new(32, window, atom, [1, x11rb::CURRENT_TIME, 0, 0, 0]);
                self.c.send_event(
                    false,
                    self.root,
                    EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                    event,
                )?;
                self.c.flush()?;
                record_usage(key);
                self.usage = load_usage();
                Ok(false)
            }
            Target::Spawn {
                argv,
                terminal,
                path,
            } => match spawn_command(&argv, path.as_deref(), terminal, &self.cfg) {
                Ok(()) => {
                    record_usage(key);
                    self.usage = load_usage();
                    Ok(false)
                }
                Err(e) => {
                    self.error = e;
                    self.confirm = None;
                    self.draw()?;
                    Ok(true)
                }
            },
        }
    }

    fn activate(&mut self) -> Result<bool> {
        let Some(index) = self.filtered.get(self.selected).copied() else {
            return Ok(true);
        };
        let target = self.items[index].target.clone();
        let key = self.items[index].key.clone();
        self.execute(&key, target)
    }

    fn close(&self) -> Result<()> {
        let _ = self.c.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.c.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self
            .c
            .set_selection_owner(x11rb::NONE, self.selection, x11rb::CURRENT_TIME);
        self.c.destroy_window(self.win)?;
        self.c.flush()?;
        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        loop {
            let event = match self.c.poll_for_event()? {
                Some(event) => event,
                None => {
                    self.c.flush()?;
                    self.c.wait_for_event()?
                }
            };
            match event {
                Event::KeyPress(e) => {
                    let (key, shift) = key_text(&self.c, e.detail, e.state);
                    if e.state.contains(KeyButMask::CONTROL) && key == 0x70 {
                        self.toggle_favorite()?;
                        continue;
                    }
                    let handled = match key {
                        0xff1b => {
                            if self.confirm.take().is_some() {
                                self.draw()?;
                            } else {
                                self.close()?;
                                return Ok(());
                            }
                            true
                        }
                        0xff0d | 0xff8d => {
                            if !self.activate()? {
                                self.close()?;
                                return Ok(());
                            }
                            true
                        }
                        0xff08 => {
                            self.query.pop();
                            self.refilter();
                            self.draw()?;
                            true
                        }
                        0xff52 | 0xff55 | 0xff50 => {
                            self.move_selection(-1);
                            self.draw()?;
                            true
                        }
                        0xff54 | 0xff56 | 0xff57 => {
                            self.move_selection(1);
                            self.draw()?;
                            true
                        }
                        0xff09 => {
                            self.move_selection(if shift { -1 } else { 1 });
                            self.draw()?;
                            true
                        }
                        0x0a => {
                            self.move_selection(1);
                            self.draw()?;
                            true
                        }
                        0x0b => {
                            self.move_selection(-1);
                            self.draw()?;
                            true
                        }
                        _ => false,
                    };
                    if !handled && is_text_key(key) {
                        if let Some(ch) = char::from_u32(key) {
                            if !ch.is_control() && !ch.is_whitespace() || ch == ' ' {
                                self.query.push(ch);
                                self.refilter();
                                self.draw()?;
                            }
                        }
                    }
                }
                Event::ButtonPress(e) => {
                    if e.event == self.win {
                        if let Some(line) = self.row_at(e.event_y) {
                            self.selected = line;
                            if !self.activate()? {
                                self.close()?;
                                return Ok(());
                            }
                        }
                    } else {
                        self.close()?;
                        return Ok(());
                    }
                }
                Event::Expose(_) => self.draw()?,
                Event::ClientMessage(e) if e.type_ == self.activate => {
                    let _ = self.c.configure_window(
                        self.win,
                        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                    );
                    let _ = self.c.set_input_focus(
                        InputFocus::POINTER_ROOT,
                        self.win,
                        x11rb::CURRENT_TIME,
                    );
                    self.c.flush()?;
                }
                Event::Error(e) => eprintln!("nirulauncher: X11 error: {e:?}"),
                Event::KeyRelease(_) | Event::ButtonRelease(_) | Event::MotionNotify(_) => {}
                _ => {}
            }
        }
    }

    fn row_at(&self, y: i16) -> Option<usize> {
        let header = self.row_height + self.padding * 2;
        if y < header {
            return None;
        }
        let line = ((y - header) / self.row_height) as usize;
        if line >= self.rows {
            return None;
        }
        let index = self.offset + line;
        (index < self.filtered.len()).then_some(index)
    }
}

fn truncate(text: &str, max_chars: i16) -> String {
    let max_chars = max_chars.max(1) as usize;
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars == 1 {
        return "…".into();
    }
    text.chars().take(max_chars - 1).collect::<String>() + "…"
}

fn spawn_command(
    argv: &[String],
    path: Option<&std::path::Path>,
    terminal: bool,
    cfg: &Config,
) -> std::result::Result<(), String> {
    if argv.is_empty() {
        return Err("empty command".into());
    }
    let mut full = if terminal {
        let mut v = cfg.terminal.clone();
        v.extend(argv.iter().cloned());
        v
    } else {
        argv.to_vec()
    };
    if full.is_empty() {
        return Err("empty command".into());
    }
    let program = full.remove(0);
    let mut command = Command::new(&program);
    command.args(&full);
    command.stdin(Stdio::null());
    if let Some(path) = path {
        command.current_dir(path);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {program}: {e}"))
}

/// Keyboard level: base, Shift, AltGr (MOD5), then Shift+AltGr.
fn is_text_key(key: u32) -> bool {
    // Latin-1 printable range covers the launcher's documented input support.
    // Modifier and function keysyms (0xfe00+) are never search text; 0xffe3
    // (Control_L) is also the printable U+FFE3 character, which caused Ctrl to
    // clear the menu.
    (0x20..=0xff).contains(&key)
}

fn keyboard_level(state: KeyButMask) -> usize {
    let shift = state.contains(KeyButMask::SHIFT);
    let altgr = state.contains(KeyButMask::MOD5);
    match (shift, altgr) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) => 3,
    }
}

/// X applies Caps Lock to alphabetic keysyms, including Latin-1 letters.
fn apply_caps(sym: u32, lock: bool) -> u32 {
    if !lock {
        return sym;
    }
    if (0x61..=0x7a).contains(&sym) || (0xe0..=0xfe).contains(&sym) && sym != 0xf7 {
        return sym.saturating_sub(0x20);
    }
    if (0x41..=0x5a).contains(&sym) || (0xc0..=0xde).contains(&sym) && sym != 0xd7 {
        return sym + 0x20;
    }
    sym
}

fn keyboard_symbol(syms: &[u32], state: KeyButMask) -> u32 {
    let level = keyboard_level(state);
    let sym = syms
        .get(level)
        .or_else(|| syms.get(level % 2))
        .or_else(|| syms.first())
        .copied()
        .unwrap_or(0);
    apply_caps(sym, state.contains(KeyButMask::LOCK))
}

fn key_text(c: &RustConnection, keycode: u8, state: KeyButMask) -> (u32, bool) {
    let shift = state.contains(KeyButMask::SHIFT);
    let Ok(map) = c
        .get_keyboard_mapping(keycode, 1)
        .ok()
        .and_then(|r| r.reply().ok())
        .ok_or(())
    else {
        return (0, shift);
    };
    let _ = map.keysyms_per_keycode;
    (keyboard_symbol(&map.keysyms, state), shift)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.first().map(String::as_str).unwrap_or("--apps");
    let result = match arg {
        "--version" => {
            println!("NIRULANCHER {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--check-config" => match Config::load() {
            Ok(cfg) => {
                println!("{}", cfg.describe());
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--list" => {
            let apps = desktop::discover();
            for app in apps {
                println!("{}\t{}", app.name, app.id);
            }
            Ok(())
        }
        "--list-actions" => match Config::load() {
            Ok(cfg) => {
                for item in action_items(&cfg) {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.group, item.name, item.detail, item.key
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--search" | "--search=" => {
            // `--search PHRASE` opens the address bar with the text already in
            // it, which is the useful thing for a keyboard shortcut that
            // passes the selection. With no phrase it starts empty.
            let phrase = args
                .get(1)
                .cloned()
                .or_else(|| arg.strip_prefix("--search=").map(str::to_string))
                .unwrap_or_default();
            match Ui::new("search") {
                Ok(mut ui) => {
                    ui.query = phrase;
                    ui.refilter();
                    ui.run()
                }
                Err(e) => Err(e),
            }
        }
        "--list-search" => match Config::load() {
            Ok(cfg) => {
                for source in websearch::sources(&cfg.search_sources) {
                    println!(
                        "{}\t{}\t{}\t{}",
                        source.label,
                        source.id,
                        source.url,
                        source.host()
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--apps" | "--actions" | "--windows" | "--microphone" | "--bluetooth" | "--displays"
        | "--power" => {
            let mode = match arg {
                "--actions" => "actions",
                "--windows" => "windows",
                "--microphone" => "microphone",
                "--bluetooth" => "bluetooth",
                "--displays" => "displays",
                "--power" => "power",
                _ => "apps",
            };
            match Ui::new(mode) {
                Ok(mut ui) => ui.run(),
                Err(e) => Err(e),
            }
        }
        "--help" => {
            println!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--windows|--microphone|--bluetooth|--displays|--power|--check-config|--list|--list-actions|--list-search|--version]"
            );
            Ok(())
        }
        _ => {
            eprintln!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--windows|--microphone|--bluetooth|--displays|--power|--check-config|--list|--list-actions|--list-search|--version]"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("nirulauncher: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod input_tests {
    use super::{apply_caps, keyboard_level, keyboard_symbol};
    use x11rb::protocol::xproto::KeyButMask;

    #[test]
    #[ignore = "requires a private X server without another window manager"]
    fn focus_waits_for_window_manager_mapping() {
        use super::*;
        let (wm, screen) = x11rb::connect(None).unwrap();
        let root = wm.setup().roots[screen].root;
        wm.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_REDIRECT),
        )
        .unwrap()
        .check()
        .unwrap();
        let (client, _) = x11rb::connect(None).unwrap();
        let win = client.generate_id().unwrap();
        client
            .create_window(
                0,
                win,
                root,
                0,
                0,
                200,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        client.map_window(win).unwrap();
        client.flush().unwrap();
        assert_eq!(
            client
                .get_window_attributes(win)
                .unwrap()
                .reply()
                .unwrap()
                .map_state,
            MapState::UNMAPPED
        );
        // Prove this fixture reproduces the original startup failure.
        assert!(client
            .set_input_focus(InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .is_err());
        let manager = std::thread::spawn(move || loop {
            if let Event::MapRequest(event) = wm.wait_for_event().unwrap() {
                assert_eq!(event.window, win);
                std::thread::sleep(std::time::Duration::from_millis(75));
                wm.map_window(win).unwrap().check().unwrap();
                return wm;
            }
        });
        super::wait_until_viewable(&client, win).unwrap();
        client
            .set_input_focus(InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            client.get_input_focus().unwrap().reply().unwrap().focus,
            win
        );
        let _wm = manager.join().unwrap();
        client.destroy_window(win).unwrap().check().unwrap();
    }

    #[test]
    fn levels_cover_shift_and_altgr() {
        assert_eq!(keyboard_level(KeyButMask::default()), 0);
        assert_eq!(keyboard_level(KeyButMask::SHIFT), 1);
        assert_eq!(keyboard_level(KeyButMask::MOD5), 2);
        assert_eq!(keyboard_level(KeyButMask::SHIFT | KeyButMask::MOD5), 3);
    }

    fn searchable(key: &str, group: &str, name: &str) -> super::Item {
        super::Item {
            key: key.into(),
            group: group.into(),
            name: name.into(),
            detail: String::new(),
            favorite: false,
            target: super::Target::Confirm(super::Action::Logout),
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
        let mut usage = std::collections::HashMap::new();
        usage.insert("c".to_string(), 20);
        usage.insert("a".to_string(), 10);
        let order = super::filtered(&items, "", &usage);
        assert_eq!(order, vec![1, 0, 2], "favourites first, then alphabetical");
    }

    #[test]
    fn group_names_are_searchable() {
        let items = vec![searchable("w1", "Windows", "Editor")];
        let usage = std::collections::HashMap::new();
        assert_eq!(super::filtered(&items, "windows", &usage), vec![0]);
        assert_eq!(super::filtered(&items, "editor", &usage), vec![0]);
        assert!(super::filtered(&items, "missing", &usage).is_empty());
    }

    #[test]
    fn action_list_includes_workspace_ten() {
        let items = super::action_items(&crate::config::Config::default());
        assert!(items.iter().any(|item| {
            item.key == "action:workspace:10"
                && item.group == "Workspaces"
                && item.name == "Workspace 10"
        }));
        assert!(items.iter().any(|item| item.key == "action:reset-bar"));
    }

    #[test]
    fn long_device_names_are_shortened() {
        assert_eq!(
            super::short_device(
                "Tiger Lake-LP Smart Sound Technology Audio Controller Digital Microphone"
            ),
            "Digital Microphone"
        );
        assert_eq!(
            super::short_device(
                "Tiger Lake-LP Smart Sound Technology Audio Controller HDMI / DisplayPort 3 Output"
            ),
            "HDMI / DisplayPort 3 Output"
        );
    }

    #[test]
    fn modifier_keys_are_not_search_text() {
        assert!(!super::is_text_key(0xffe3), "Ctrl is not text");
        assert!(!super::is_text_key(0xffe1), "Shift is not text");
        assert!(!super::is_text_key(0xff52), "Arrow is not text");
        assert!(super::is_text_key(0x70), "p is text");
        assert!(super::is_text_key(0xe5), "å is text");
    }

    #[test]
    fn altgr_and_caps_produce_the_expected_symbols() {
        // Swedish key with base, Shift, AltGr and Shift+AltGr levels.
        let syms = [0x32, 0x22, 0x40, 0x32];
        assert_eq!(keyboard_symbol(&syms, KeyButMask::MOD5), 0x40);
        assert_eq!(
            keyboard_symbol(&syms, KeyButMask::SHIFT | KeyButMask::MOD5),
            0x32
        );
        assert_eq!(apply_caps(0xe5, true), 0xc5, "Caps Lock applies to å");
        assert_eq!(apply_caps(0x40, true), 0x40, "Caps Lock leaves @ alone");
    }
}

#[cfg(test)]
mod wallpaper_tests {
    use super::{action_items, filtered, Config};

    fn items_with_wallpaper(installed: bool) -> Vec<super::Item> {
        let dir = std::env::temp_dir().join(format!("nl-wp-{}-{}", installed, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if installed {
            std::fs::write(dir.join("niruwallpaper"), "#!/bin/sh\n").unwrap();
        }
        let previous = std::env::var_os("PATH");
        let home = std::env::var_os("HOME");
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        // A private configuration keeps the wallpaper status of the host out
        // of this test; niructl is not installed in the sandbox anyway.
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::env::set_var("PATH", &dir);
        // `in_path` also looks in the home directories, so point HOME at the
        // same sandbox; otherwise a real installation would hide the result.
        std::env::set_var("HOME", &dir);
        let items = action_items(&Config::default());
        match previous {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        match home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        std::fs::remove_dir_all(&dir).ok();
        items
    }

    /// The environment is process-wide, so these tests run one at a time.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn the_wallpaper_group_is_one_settings_entry() {
        let _guard = lock();
        let items = items_with_wallpaper(true);
        let rows: Vec<_> = items.iter().filter(|i| i.group == "Wallpaper").collect();
        let keys: Vec<&str> = rows.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            rows.len(),
            1,
            "only the settings window belongs here: {keys:?}"
        );
        assert_eq!(rows[0].key, "action:wallpaper:open");
        // The image, scale, rotation and follow controls moved into the window.
        for key in [
            "action:wallpaper:mode",
            "action:wallpaper:rotation",
            "action:wallpaper:scale",
            "action:wallpaper:follow-theme",
            "action:wallpaper:download-all",
        ] {
            assert!(
                !items.iter().any(|i| i.key == key),
                "{key} should not be a launcher row"
            );
        }
    }

    #[test]
    fn no_wallpaper_row_without_the_window() {
        let _guard = lock();
        let items = items_with_wallpaper(false);
        assert!(!items.iter().any(|i| i.group == "Wallpaper"));
    }

    #[test]
    fn themes_stay_in_the_launcher() {
        let _guard = lock();
        let items = items_with_wallpaper(true);
        assert!(
            items.iter().any(|i| i.key.starts_with("action:theme:")),
            "themes are applied from the launcher"
        );
    }

    #[test]
    fn the_settings_row_is_searchable() {
        let _guard = lock();
        let items = items_with_wallpaper(true);
        let usage = std::collections::HashMap::new();
        for query in ["wallpaper", "background", "appearance"] {
            assert!(
                filtered(&items, query, &usage)
                    .iter()
                    .any(|&index| items[index].key == "action:wallpaper:open"),
                "{query} must find the settings window"
            );
        }
    }
}

#[cfg(test)]
mod nirux_update_tests {
    use super::{installed_nirux_version, is_newer_version, nirux_update_available};
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    struct Sandbox {
        dir: std::path::PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Sandbox {
        fn new(name: &str) -> Self {
            let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir().join(format!("nl-nux-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("HOME", &dir);
            std::env::set_var("XDG_CACHE_HOME", dir.join(".cache"));
            Self { dir, _guard: guard }
        }

        fn install(&self, version: &str) {
            let gen = self
                .dir
                .join(format!(".local/share/nirux/generations/{version}-X"));
            std::fs::create_dir_all(&gen).unwrap();
            std::os::unix::fs::symlink(&gen, self.dir.join(".local/share/nirux/current")).unwrap();
        }

        fn cache(&self, latest: &str) {
            let cache = self.dir.join(".cache/nirubar");
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(cache.join("nirux-update"), latest).unwrap();
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer_version("0.12.2", "0.12.1"));
        assert!(is_newer_version("0.13.0", "0.12.99"));
        assert!(!is_newer_version("0.12.1", "0.12.1"));
        assert!(!is_newer_version("0.9.0", "0.10.0"));
    }

    #[test]
    fn the_installed_version_comes_from_the_generation() {
        let box_ = Sandbox::new("installed");
        box_.install("0.12.1");
        assert_eq!(installed_nirux_version().as_deref(), Some("0.12.1"));
    }

    #[test]
    fn a_newer_cached_release_offers_the_action() {
        let box_ = Sandbox::new("newer");
        box_.install("0.12.1");
        box_.cache("0.12.2");
        assert_eq!(nirux_update_available().as_deref(), Some("0.12.2"));
    }

    #[test]
    fn an_up_to_date_machine_has_no_action() {
        let box_ = Sandbox::new("current");
        box_.install("0.12.2");
        box_.cache("0.12.2");
        assert_eq!(nirux_update_available(), None);
    }

    #[test]
    fn a_machine_without_nirux_has_no_action() {
        let _box = Sandbox::new("nosuite");
        assert_eq!(nirux_update_available(), None);
    }
}

#[cfg(test)]
mod search_tests {
    use super::{filtered, Item, Target};

    fn row(group: &str, name: &str, detail: &str) -> Item {
        Item {
            key: format!("{group}:{name}"),
            group: group.into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: Target::Info,
        }
    }

    /// A row is found by its group name, because the group is part of what the
    /// list calls it. Several groups rely on this instead of repeating the
    /// word in every row's name.
    #[test]
    fn a_row_is_found_by_its_group_name() {
        let items = vec![
            row("Appearance", "Compositor", "Toggle transparency"),
            row("Files", "Backup", "Copy to a disk"),
        ];
        let usage = std::collections::HashMap::new();
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
        let usage = std::collections::HashMap::new();
        let order = filtered(&items, "wallpaper", &usage);
        assert_eq!(
            items[order[0]].name, "Wallpaper",
            "the shorter label is the closer match"
        );
    }

    /// Typing a phrase in `--search` puts the engine rows above the local
    /// matches, and the first one is a complete, ready-to-open address.
    #[test]
    fn a_phrase_offers_every_engine_at_the_top() {
        // A local match that contains the phrase, so ranking and the engine
        // rows are compared together.
        let mut plain = vec![row("App", "black metal 1994 playlist", "music")];
        let before = plain.len();
        let sources = super::websearch::sources(&[]);
        let phrase = "black metal 1994";
        for source in &sources {
            plain.push(super::websearch::item(source, phrase).expect("a row per engine"));
        }
        let usage = std::collections::HashMap::new();
        let order = filtered(&plain, phrase, &usage);
        // In the running launcher the engine rows are placed above the local
        // hits whatever their rank; `filtered` sorts the local ones only.
        let first_engine = &plain[order[0]];
        assert_eq!(first_engine.name, "black metal 1994 playlist");
        let first_engine = &plain[before];
        assert_eq!(first_engine.name, "DuckDuckGo: black metal 1994");
        match &first_engine.target {
            Target::Spawn { argv, .. } => {
                assert!(argv.last().unwrap().contains("black+metal+1994"));
            }
            _ => panic!("a search row must open a program"),
        }
        // Every engine got a row, and none of them hides a local match.
        let engine_rows = plain[before..]
            .iter()
            .filter(|item| item.group == "Search")
            .count();
        assert_eq!(engine_rows, sources.len());
    }

    /// Three letters is the point where a filter becomes a search.
    #[test]
    fn short_text_is_not_a_web_search() {
        assert_eq!(super::MIN_SEARCH_CHARS, 3);
        let sources = super::websearch::sources(&[]);
        let short = "ab";
        assert!(sources
            .iter()
            .filter_map(|source| super::websearch::item(source, short))
            .all(|item| item.name.ends_with("ab")));
    }
}
