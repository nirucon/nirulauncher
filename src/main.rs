//! NIRULANCHER — native application and desktop-action launcher.
//!
//! Keyboard-driven, one process per invocation. No indexing daemon, no network
//! and no shell: applications are started from argv arrays parsed from their
//! desktop entries.

mod config;
mod desktop;
mod intent;
mod render;
mod saved;
mod sound;
mod websearch;
mod wifi;

use config::Config;
use render::Renderer;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
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
    ForgetWifi(String),
}

#[derive(Clone)]
enum Target {
    Spawn {
        argv: Vec<String>,
        terminal: bool,
        path: Option<std::path::PathBuf>,
    },
    /// Run in the configured terminal. `hold` keeps the window open after the
    /// child exits so reports and diagnostics can actually be read.
    Terminal {
        argv: Vec<String>,
        hold: bool,
    },
    Focus(Window),
    Confirm(Action),
    /// Switch the open launcher to another view without starting a second one.
    Mode(String),
    /// Run an explicit microphone test and show the result in the sound panel.
    SoundTest(u64),
    Info,
}

#[derive(Clone)]
struct Item {
    /// Stable key for favourites and local usage.
    key: String,
    group: String,
    name: String,
    detail: String,
    /// Extra searchable words from a desktop entry (Keywords, comment).
    keywords: String,
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

    #[cfg(test)]
    fn target_kind(&self) -> &'static str {
        match self.target {
            Target::Spawn { .. } => "spawn",
            Target::Terminal { .. } => "terminal",
            Target::Focus(_) => "focus",
            Target::Confirm(_) => "confirm",
            Target::Mode(_) => "mode",
            Target::SoundTest(_) => "sound-test",
            Target::Info => "info",
        }
    }
}

/// An inline numeric edit for the idle/lock timing view.
#[derive(Clone)]
struct NumberEdit {
    key: String,
    label: String,
    buffer: String,
}

/// A Wi-Fi password or hidden-network prompt. Secrets stay in memory only.
#[derive(Clone)]
struct WifiSecret {
    ssid: String,
    security: String,
    buffer: String,
    hidden: bool,
    stage: u8,
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
    /// The current view, so an ordered list can keep its intended order.
    mode: String,
    /// The effective providers: order, aliases and the configured default.
    catalog: websearch::Catalog,
    /// How many items existed before the mode added its own rows.
    plain_items: usize,
    /// The profile name while a layout is being saved, with the cursor.
    profile_name: Option<String>,
    /// The minutes field while an idle/lock timing value is being edited.
    number_edit: Option<NumberEdit>,
    /// Latest Wi-Fi state, plus one in-flight backend operation.
    wifi: Option<wifi::Snapshot>,
    wifi_rx: Option<Receiver<std::result::Result<wifi::Snapshot, String>>>,
    wifi_busy: Option<String>,
    wifi_secret: Option<WifiSecret>,
    sound: Option<sound::Snapshot>,
    sound_rx: Option<Receiver<std::result::Result<sound::Snapshot, String>>>,
    sound_test_rx: Option<Receiver<std::result::Result<sound::TestResult, String>>>,
    sound_test: Option<sound::TestResult>,
    sound_busy: Option<String>,
}

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
            keywords: String::new(),
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
                    keywords: String::new(),
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

fn sound_info(name: &str, detail: &str) -> Item {
    let key = format!(
        "sound:info:{}",
        name.to_lowercase()
            .chars()
            .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
            .collect::<String>()
    );
    Item {
        key,
        group: "Audio".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: "sound audio volume mute microphone mic".into(),
        favorite: false,
        target: Target::Info,
    }
}

fn sound_items(
    snapshot: Option<&sound::Snapshot>,
    test: Option<&sound::TestResult>,
    busy: Option<&str>,
) -> Vec<Item> {
    if let Some(busy) = busy {
        return vec![sound_info(busy, "Working…")];
    }
    let Some(snapshot) = snapshot else {
        return vec![sound_info("Loading audio…", "Reading PipeWire state")];
    };
    if !snapshot.available {
        return vec![sound_info(
            "Audio unavailable",
            snapshot
                .reason
                .as_deref()
                .unwrap_or("PipeWire is not responding"),
        )];
    }
    let mut items = Vec::new();
    let output = snapshot.output.as_ref();
    items.push(sound_info(
        &format!(
            "Output · {}",
            output
                .map(|row| short_device(&row.name))
                .unwrap_or_else(|| "No output".into())
        ),
        &output
            .map(|row| {
                format!(
                    "{}%{}",
                    row.percent,
                    if row.muted { " · muted" } else { "" }
                )
            })
            .unwrap_or_else(|| "No default output".into()),
    ));
    items.push(Item {
        key: "sound:volume-up".into(),
        group: "Output".into(),
        name: "Volume +5%".into(),
        detail: "Raise the default output volume".into(),
        keywords: "sound audio volume up louder".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "volume", "up"]),
    });
    items.push(Item {
        key: "sound:volume-down".into(),
        group: "Output".into(),
        name: "Volume −5%".into(),
        detail: "Lower the default output volume".into(),
        keywords: "sound audio volume down quieter".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "volume", "down"]),
    });
    items.push(Item {
        key: "sound:output-mute".into(),
        group: "Output".into(),
        name: "Mute / unmute output".into(),
        detail: "Toggle the default output · niructl audio mute".into(),
        keywords: "sound audio mute output speaker".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "mute"]),
    });
    for node in &snapshot.outputs {
        items.push(Item {
            key: format!("sound:output:{}", node.id),
            group: "Output".into(),
            name: format!("Use output · {}", short_device(&node.name)),
            detail: if node.default {
                "Current output"
            } else {
                "Select this output"
            }
            .into(),
            keywords: "sound audio output device speaker".into(),
            favorite: false,
            target: spawn_niructl(&["audio", "output", &node.id.to_string()]),
        });
    }
    let input = snapshot.input.as_ref();
    items.push(sound_info(
        &format!(
            "Input · {}",
            input
                .map(|row| short_device(&row.name))
                .unwrap_or_else(|| "No input".into())
        ),
        &input
            .map(|row| {
                format!(
                    "{}%{}",
                    row.percent,
                    if row.muted { " · muted" } else { "" }
                )
            })
            .unwrap_or_else(|| "No default microphone".into()),
    ));
    items.push(Item {
        key: "sound:mic-up".into(),
        group: "Input".into(),
        name: "Microphone level +5%".into(),
        detail: "Raise input gain without unmuting · niructl audio mic-level up".into(),
        keywords: "sound audio microphone mic gain level up".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "mic-level", "up"]),
    });
    items.push(Item {
        key: "sound:mic-down".into(),
        group: "Input".into(),
        name: "Microphone level −5%".into(),
        detail: "Lower input gain without unmuting · niructl audio mic-level down".into(),
        keywords: "sound audio microphone mic gain level down".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "mic-level", "down"]),
    });
    items.push(Item {
        key: "sound:mic-mute".into(),
        group: "Input".into(),
        name: "Mute / unmute microphone".into(),
        detail: "Toggle input mute · niructl audio mic-mute toggle".into(),
        keywords: "sound audio microphone mic mute".into(),
        favorite: false,
        target: spawn_niructl(&["audio", "mic-mute", "toggle"]),
    });
    items.push(Item {
        key: "sound:mic-test".into(),
        group: "Input".into(),
        name: "Test microphone…".into(),
        detail: "Record five seconds locally and measure peak/RMS".into(),
        keywords: "sound audio microphone mic test record level".into(),
        favorite: false,
        target: Target::SoundTest(5),
    });
    for node in &snapshot.sources {
        items.push(Item {
            key: format!("sound:input:{}", node.id),
            group: "Input".into(),
            name: format!("Use input · {}", short_device(&node.name)),
            detail: if node.default {
                "Current input"
            } else {
                "Select this input"
            }
            .into(),
            keywords: "sound audio input microphone device".into(),
            favorite: false,
            target: spawn_niructl(&["audio", "input", &node.id.to_string()]),
        });
    }
    if let Some(result) = test {
        let clip = if result.clipping {
            "clipping"
        } else {
            "no clipping"
        };
        items.push(sound_info(
            &format!("Last test · {}", result.verdict),
            &format!(
                "Peak {:.1} dBFS · RMS {:.1} dBFS · {clip}",
                result.peak_dbfs, result.rms_dbfs
            ),
        ));
        if let Some(file) = &result.file {
            items.push(Item {
                key: "sound:test-play".into(),
                group: "Input".into(),
                name: "Play test recording".into(),
                detail: "Local temporary WAV · deleted when this panel closes".into(),
                keywords: "sound audio microphone test listen playback".into(),
                favorite: false,
                target: Target::Spawn {
                    argv: vec!["pw-play".into(), file.clone()],
                    terminal: false,
                    path: None,
                },
            });
        }
    }
    if !snapshot.streams.is_empty() {
        for stream in &snapshot.streams {
            items.push(sound_info(
                &format!("Application · {}", short_device(&stream.name)),
                &format!("Active audio stream · id {}", stream.id),
            ));
        }
    } else {
        items.push(sound_info("Applications", "No active audio streams"));
    }
    items
}

fn wifi_items(snapshot: &wifi::Snapshot) -> Vec<Item> {
    let mut items = Vec::new();
    let group = "Wi-Fi";
    if !snapshot.present {
        return vec![Item {
            key: "wifi:absent".into(),
            group: group.into(),
            name: "No Wi-Fi hardware".into(),
            detail: "This computer has no wireless adapter".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }];
    }
    if snapshot.hard_blocked {
        items.push(wifi_info(
            "Wi-Fi is blocked by hardware",
            "Use the physical wireless switch or airplane-mode key",
        ));
    } else if snapshot.soft_blocked || !snapshot.powered {
        items.push(Item {
            key: "wifi:power".into(),
            group: group.into(),
            name: "Turn Wi-Fi on".into(),
            detail: if snapshot.hard_blocked {
                "Hardware blocked"
            } else {
                "Currently off"
            }
            .into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    } else {
        items.push(Item {
            key: "wifi:power".into(),
            group: group.into(),
            name: "Turn Wi-Fi off".into(),
            detail: format!("{} · on", snapshot.interface),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }

    if snapshot.connected {
        let signal = snapshot
            .signal
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "signal unknown".into());
        let detail = if snapshot.address.is_empty() {
            format!("{signal} · {}", snapshot.security)
        } else {
            format!("{signal} · {} · {}", snapshot.security, snapshot.address)
        };
        items.push(Item {
            key: "wifi:connected".into(),
            group: group.into(),
            name: format!("Connected: {}", snapshot.ssid),
            detail,
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: "wifi:disconnect".into(),
            group: group.into(),
            name: format!("Disconnect from {}", snapshot.ssid),
            detail: "Disconnect this Wi-Fi link; keep the saved profile".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        if snapshot.saved.contains(&snapshot.ssid) && snapshot.active_profile.is_empty() {
            items.push(Item {
                key: format!("wifi:forget:{}", snapshot.ssid),
                group: group.into(),
                name: format!("Forget {}", snapshot.ssid),
                detail: "Remove the saved profile".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
    }

    for profile in &snapshot.profiles {
        items.push(Item {
            key: format!("wifi:profile:{}", profile.id),
            group: group.into(),
            name: format!("Saved: {}", profile.ssid),
            detail: format!(
                "{} · {}{}",
                profile.security,
                profile.id.rsplit('/').next().unwrap_or("profile"),
                if profile.id == snapshot.active_profile {
                    " · active"
                } else {
                    ""
                }
            ),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: format!("wifi:forget:{}", profile.id),
            group: group.into(),
            name: format!("Forget saved profile: {}", profile.ssid),
            detail: profile.id.clone(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }

    for network in &snapshot.networks {
        if network.active {
            continue;
        }
        let signal = network
            .signal
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "not in range".into());
        let mut detail = format!("{signal} · {}", network.security);
        if network.saved {
            detail.push_str(" · saved");
        }
        if network.security.contains("Enterprise") {
            detail.push_str(" · saved profile required");
        }
        items.push(Item {
            key: format!("wifi:connect:{}", network.ssid),
            group: group.into(),
            name: network.ssid.clone(),
            detail,
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        if network.saved && network.needs_secret() {
            items.push(Item {
                key: format!("wifi:connect-pw:{}", network.ssid),
                group: group.into(),
                name: format!("Use password for {}", network.ssid),
                detail: "Saved profile · enter a new password".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
        if network.saved && snapshot.manager != "NetworkManager" {
            items.push(Item {
                key: format!("wifi:forget:{}", network.ssid),
                group: group.into(),
                name: format!("Forget {}", network.ssid),
                detail: "Remove the saved profile".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
    }

    if snapshot.powered && !snapshot.hard_blocked && !snapshot.soft_blocked {
        items.push(Item {
            key: "wifi:hidden-wpa".into(),
            group: group.into(),
            name: "Hidden network…".into(),
            detail: "Enter an SSID and WPA2/WPA3 password".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
        items.push(Item {
            key: "wifi:hidden-open".into(),
            group: group.into(),
            name: "Hidden open network…".into(),
            detail: "Enter an SSID; no password".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    items.push(Item {
        key: "wifi:refresh".into(),
        group: group.into(),
        name: "Rescan networks".into(),
        detail: "Look for networks again".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    });
    if items.is_empty() {
        items.push(wifi_info(
            "No networks found",
            "Move closer to an access point or rescan",
        ));
    }
    items
}

/// Search SSIDs, not command labels, security text or launcher favourites.
fn wifi_filtered(items: &[Item], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (query.is_empty()
                || ((item.key.starts_with("wifi:connect:")
                    || item.key.starts_with("wifi:profile:")
                    || item.key == "wifi:connected")
                    && item.name.to_lowercase().contains(&query)))
            .then_some(index)
        })
        .collect()
}

fn wifi_info(name: &str, detail: &str) -> Item {
    Item {
        key: format!("wifi:info:{name}"),
        group: "Wi-Fi".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }
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
        keywords: String::new(),
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
            keywords: String::new(),
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
            keywords: String::new(),
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
            keywords: String::new(),
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
            keywords: String::new(),
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
            keywords: String::new(),
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
    let latest = std::fs::read_to_string(&cache).ok()?.trim().to_string();
    let installed = installed_nirux_version()?;
    is_newer_version(&latest, &installed).then(|| {
        // The answer is worth showing once. Keeping it would offer the same
        // update after it has been installed, until the bar checks again.
        let _ = std::fs::remove_file(&cache);
        latest
    })
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

/// Run `niructl action list --json` with a bounded wait and bounded output.
/// The error string explains why the helper could not provide a list; normal
/// loading ignores it so an older or missing helper simply yields no rows.
fn load_saved_actions_result() -> std::result::Result<Vec<saved::SavedAction>, String> {
    let mut child = match Command::new("niructl")
        .args(["action", "list", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("niructl was not found".into());
        }
        Err(error) => return Err(format!("could not start niructl: {error}")),
    };
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut timed_out = false;
    let mut success = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                success = status.success();
                break;
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            _ => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
        }
    }
    let mut bytes = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout
            .take(saved::MAX_JSON_BYTES as u64 + 1)
            .read_to_end(&mut bytes);
    }
    let _ = child.wait();
    if timed_out {
        return Err("niructl action list timed out".into());
    }
    if !success {
        return Err("this niructl has no working action command".into());
    }
    Ok(saved::parse(&bytes))
}

fn load_saved_actions() -> Vec<saved::SavedAction> {
    load_saved_actions_result().unwrap_or_default()
}

fn saved_action_item(action: saved::SavedAction) -> Item {
    let command = action
        .command
        .iter()
        .map(|arg| {
            arg.chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ");
    Item {
        key: format!("saved:action:{}", action.name.to_lowercase()),
        group: "Saved actions".into(),
        name: action.name,
        detail: format!("Saved command · {command}"),
        keywords: "saved action command".into(),
        favorite: action.favorite,
        target: Target::Spawn {
            argv: action.command,
            terminal: false,
            path: None,
        },
    }
}

/// Append saved rows without taking over an existing key.
fn append_saved_actions(items: &mut Vec<Item>, actions: &[saved::SavedAction]) {
    let mut keys: std::collections::HashSet<String> =
        items.iter().map(|item| item.key.clone()).collect();
    for action in actions {
        let item = saved_action_item(action.clone());
        if keys.insert(item.key.clone()) {
            items.push(item);
        }
    }
}

fn intent_terminal(args: &[&str]) -> Target {
    Target::Terminal {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|arg| (*arg).to_string()))
            .collect(),
        hold: true,
    }
}

fn intent_item(found: &intent::IntentMatch, nmcli: bool) -> Item {
    use intent::IntentAction as I;
    let (name, detail, target) = match found.action {
        I::Lock => (
            "Lock screen",
            "Lock now · niructl lock",
            spawn_niructl(&["lock"]),
        ),
        I::Logout => (
            "Log out",
            "Log out of this session · confirmation required",
            Target::Confirm(Action::Logout),
        ),
        I::Reboot => (
            "Restart computer",
            "Reboot now · confirmation required",
            Target::Confirm(Action::Reboot),
        ),
        I::Shutdown => (
            "Shut down",
            "Power off now · confirmation required",
            Target::Confirm(Action::Shutdown),
        ),
        I::FocusMode => (
            "Focus mode for 60 minutes",
            "DND and keep-awake · niructl mode focus 60",
            spawn_niructl(&["mode", "focus", "60"]),
        ),
        I::PresentationMode => (
            "Presentation mode for 60 minutes",
            "DND and keep-awake · niructl mode presentation 60",
            spawn_niructl(&["mode", "presentation", "60"]),
        ),
        I::StopContext => (
            "Stop work context",
            "Restore DND and idle state · niructl mode stop",
            spawn_niructl(&["mode", "stop"]),
        ),
        I::Dnd => (
            "Toggle notifications",
            "Pause or resume DND · niructl dnd toggle",
            spawn_niructl(&["dnd", "toggle"]),
        ),
        I::KeepAwake => (
            "Toggle keep-awake",
            "Inhibit idle lock and DPMS · niructl idle toggle",
            spawn_niructl(&["idle", "toggle"]),
        ),
        I::VolumeUp => (
            "Volume up",
            "Raise output volume by 5% · niructl audio volume up",
            spawn_niructl(&["audio", "volume", "up"]),
        ),
        I::VolumeDown => (
            "Volume down",
            "Lower output volume by 5% · niructl audio volume down",
            spawn_niructl(&["audio", "volume", "down"]),
        ),
        I::VolumeMute => (
            "Mute audio",
            "Toggle the default output · niructl audio mute",
            spawn_niructl(&["audio", "mute"]),
        ),
        I::MicMute => (
            "Mute microphone",
            "Toggle input mute · niructl audio mic-mute toggle",
            spawn_niructl(&["audio", "mic-mute", "toggle"]),
        ),
        I::BrightnessUp => (
            "Brightness up",
            "Increase the backlight by 5% · niructl brightness up",
            spawn_niructl(&["brightness", "up"]),
        ),
        I::BrightnessDown => (
            "Brightness down",
            "Decrease the backlight by 5% · niructl brightness down",
            spawn_niructl(&["brightness", "down"]),
        ),
        I::NightLight => (
            "Night light",
            "Toggle the manual night light · niructl nightlight toggle",
            spawn_niructl(&["nightlight", "toggle"]),
        ),
        I::Compositor => (
            "Compositor",
            "Toggle picom transparency and shadows · niructl compositor toggle",
            spawn_niructl(&["compositor", "toggle"]),
        ),
        I::WindowsRescue => (
            "Bring off-screen windows back",
            "Move managed windows onto a connected display · niructl windows rescue",
            intent_terminal(&["windows", "rescue"]),
        ),
        I::WifiOn if nmcli => (
            "Turn Wi-Fi on",
            "NetworkManager radio · nmcli radio wifi on",
            Target::Spawn {
                argv: vec!["nmcli".into(), "radio".into(), "wifi".into(), "on".into()],
                terminal: false,
                path: None,
            },
        ),
        I::WifiOff if nmcli => (
            "Turn Wi-Fi off",
            "NetworkManager radio · nmcli radio wifi off",
            Target::Spawn {
                argv: vec!["nmcli".into(), "radio".into(), "wifi".into(), "off".into()],
                terminal: false,
                path: None,
            },
        ),
        I::WifiOn | I::WifiOff => (
            if found.action == I::WifiOn {
                "Turn Wi-Fi on"
            } else {
                "Turn Wi-Fi off"
            },
            "Unavailable · NetworkManager's nmcli was not found",
            Target::Info,
        ),
    };
    Item {
        key: format!("intent:{}", found.action.id()),
        group: "Intent".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: format!("intent {}", found.phrase),
        favorite: false,
        target,
    }
}

fn intent_items(query: &str) -> Vec<Item> {
    let nmcli = in_path("nmcli");
    intent::resolve(query)
        .into_iter()
        .map(|found| intent_item(&found, nmcli))
        .collect()
}

fn display_items() -> Vec<Item> {
    let mut items = Vec::new();
    if in_path("nirudisplay") {
        items.push(Item {
            key: "display:open".into(),
            group: "Displays".into(),
            name: "Open display settings".into(),
            detail: "Move screens, choose modes and rates, save profiles".into(),
            keywords: "display screen monitor arrangement refresh rate profile".into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["nirudisplay".to_string()],
                terminal: false,
                path: None,
            },
        });
    }
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
                    keywords: String::new(),
                    favorite: false,
                    target: spawn_niructl(&["display", "preview", fields[0]]),
                });
            }
        }
    }
    // Arranging the screens needs no choice, so it is one row. Rotating a
    // single screen below is the common fix for a display mounted upside down.
    items.push(Item {
        key: "display:auto".into(),
        group: "Displays".into(),
        name: "Displays: Arrange side by side".into(),
        detail: "Preferred mode on every connected screen".into(),
        keywords: String::new(),
        favorite: false,
        target: spawn_niructl(&["display", "auto"]),
    });
    // Naming the layout happens here, so saving needs no terminal.
    items.push(Item {
        key: "display:save".into(),
        group: "Displays".into(),
        name: "Displays: Save this layout as a profile".into(),
        detail: "Type a name, then Enter".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    });
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
                        keywords: String::new(),
                        favorite: false,
                        target: spawn_niructl(&["display", "rotate", output_name, rotation]),
                    });
                }
            }
        }
    }
    if let Ok(output) = Command::new("niructl")
        .args(["display", "status", "--json"])
        .output()
    {
        let token = serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .ok()
            .and_then(|state| {
                state
                    .get("preview")
                    .and_then(|preview| preview.get("token"))
                    .and_then(|token| token.as_str())
                    .map(str::to_owned)
            });
        if let Some(token) = token {
            items.push(Item {
                key: "display:keep".into(),
                group: "Displays".into(),
                name: "Keep display layout".into(),
                detail: "Confirm the preview".into(),
                keywords: String::new(),
                favorite: false,
                target: spawn_niructl(&["display", "keep", &token]),
            });
            items.push(Item {
                key: "display:revert".into(),
                group: "Displays".into(),
                name: "Revert display layout".into(),
                detail: "Return to the previous layout".into(),
                keywords: String::new(),
                favorite: false,
                target: spawn_niructl(&["display", "revert", &token]),
            });
        }
    }
    if items.is_empty() {
        items.push(Item {
            key: "display:none".into(),
            group: "Displays".into(),
            name: "No display profiles".into(),
            detail: "Create one with niructl display save NAME".into(),
            keywords: String::new(),
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
        keywords: String::new(),
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
        keywords: String::new(),
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
        "launcher_search" => "Web search",
        "launcher_unified" => "Actions and layouts",
        "launcher_actions" => "NIRU system actions",
        "window_switcher" => "Window switcher",
        "lock" => "Lock screen",
        "suspend" => "Lock and suspend",
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
        "move_next" | "swap_next" => "Swap with next window",
        "move_prev" | "swap_prev" => "Swap with previous window",
        "inc_master" => "Increase master count",
        "dec_master" => "Decrease master count",
        "inc_master_ratio" => "Increase master ratio",
        "dec_master_ratio" => "Decrease master ratio",
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
            if let Some(rest) = action.strip_prefix("scroll_columns ") {
                return format!("Scroll columns: {rest}");
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

fn theme_items() -> Vec<Item> {
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };
    let mut items = Vec::new();
    if in_path("niruthemeselector") {
        items.push(Item {
            key: "theme:open".into(),
            group: "Appearance".into(),
            name: "Open theme selector".into(),
            detail: "Preview colours and wallpaper before applying".into(),
            keywords: "theme appearance selector preview wallpaper colour".into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["niruthemeselector".to_string()],
                terminal: false,
                path: None,
            },
        });
    }
    let mut catalog = None;
    if let Ok(output) = Command::new("niructl").args(["theme-catalog"]).output() {
        if output.status.success() {
            catalog = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok();
        }
    }
    if let Some(catalog) = catalog {
        let active = catalog["active"].as_str().unwrap_or("");
        if let Some(themes) = catalog["themes"].as_array() {
            for theme in themes {
                let id = theme["id"].as_str().unwrap_or("");
                let name = theme["name"].as_str().unwrap_or(id);
                if id.is_empty() || theme["hidden"].as_bool().unwrap_or(false) {
                    continue;
                }
                if !theme["valid"].as_bool().unwrap_or(true) {
                    continue;
                }
                let origin = if theme["overridden"].as_bool().unwrap_or(false) {
                    "built-in override"
                } else if theme["builtin"].as_bool().unwrap_or(false) {
                    "built-in"
                } else {
                    "user"
                };
                items.push(Item {
                    key: format!("action:theme:{id}"),
                    group: "Appearance".into(),
                    name: format!("Theme: {name}"),
                    detail: if id == active {
                        format!("active · {origin}")
                    } else {
                        format!("apply across WM, bar, saver and lock · {origin}")
                    },
                    keywords: format!("theme appearance {id} {name}"),
                    favorite: false,
                    target: niructl(&["theme", id]),
                });
            }
        }
    } else {
        // A launcher installed before NIRUX, or a broken catalogue, still gets
        // the built-in names instead of an empty Appearance list.
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
                keywords: format!("theme appearance {theme}"),
                favorite: false,
                target: niructl(&["theme", theme]),
            });
        }
    }
    items
}

/// The active shortcut catalogue, read from NIRUWM.
///
/// This includes user overrides and skips explicit unbinds; the launcher never
/// invents a second truth that can drift from the window manager.
fn shortcut_items() -> Vec<Item> {
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
    let mut items: Vec<Item> = shortcuts
        .into_iter()
        .map(|(action, keys)| Item {
            key: format!("shortcut:{action}"),
            group: "Shortcuts".into(),
            name: shortcut_action_label(&action),
            detail: keys.join(" / "),
            keywords: format!("keybinding shortcut {action}"),
            favorite: false,
            target: Target::Info,
        })
        .collect();
    items.push(Item {
        key: "shortcut:mouse".into(),
        group: "Shortcuts".into(),
        name: "Mouse move / resize".into(),
        detail: "Super+left / right drag".into(),
        keywords: "keybinding mouse pointer".into(),
        favorite: false,
        target: Target::Info,
    });
    if items.len() == 1 {
        items.push(Item {
            key: "shortcut:none".into(),
            group: "Shortcuts".into(),
            name: "Keybindings unavailable".into(),
            detail: "niructl did not return the active bindings".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        });
    }
    items
}

/// One place that opens the settings that already exist elsewhere.
fn settings_items() -> Vec<Item> {
    let mode = |name: &str| Target::Mode(name.to_string());
    let themes_target = if in_path("niruthemeselector") {
        Target::Spawn {
            argv: vec!["niruthemeselector".to_string()],
            terminal: false,
            path: None,
        }
    } else {
        mode("themes")
    };
    let displays_target = if in_path("nirudisplay") {
        Target::Spawn {
            argv: vec!["nirudisplay".to_string()],
            terminal: false,
            path: None,
        }
    } else {
        mode("displays")
    };
    let mut items = vec![
        Item {
            key: "settings:themes".into(),
            group: "NIRUX Settings".into(),
            name: "Appearance and themes".into(),
            detail: if in_path("niruthemeselector") {
                "Graphical theme selector with previews".into()
            } else {
                "Apply a theme across the desktop".into()
            },
            keywords: "settings appearance theme colours wallpaper".into(),
            favorite: false,
            target: themes_target,
        },
        Item {
            key: "settings:displays".into(),
            group: "NIRUX Settings".into(),
            name: "Displays".into(),
            detail: if in_path("nirudisplay") {
                "Graphical monitor editor and saved profiles".into()
            } else {
                "Profiles, rotation and arrangement".into()
            },
            keywords: "settings display screen monitor profile".into(),
            favorite: false,
            target: displays_target,
        },
        Item {
            key: "settings:audio".into(),
            group: "NIRUX Settings".into(),
            name: "Audio".into(),
            detail: "Outputs, microphone and mute".into(),
            keywords: "settings sound audio microphone output".into(),
            favorite: false,
            target: mode("microphone"),
        },
        Item {
            key: "settings:keys".into(),
            group: "NIRUX Settings".into(),
            name: "Keybindings".into(),
            detail: "Every active shortcut".into(),
            keywords: "settings keyboard keybindings shortcuts".into(),
            favorite: false,
            target: mode("keys"),
        },
        Item {
            key: "settings:input".into(),
            group: "NIRUX Settings".into(),
            name: "Input devices".into(),
            detail: "Keyboard layout, repeat, touchpad and pointer".into(),
            keywords: "settings input keyboard mouse touchpad pointer layout".into(),
            favorite: false,
            target: mode("input"),
        },
        Item {
            key: "settings:timing".into(),
            group: "NIRUX Settings".into(),
            name: "Idle and lock timing".into(),
            detail: "Screen saver, lock and display off".into(),
            keywords: "settings idle lock screensaver timing minutes".into(),
            favorite: false,
            target: mode("timing"),
        },
    ];
    if wifi::hardware_present() {
        items.insert(
            2,
            Item {
                key: "settings:wifi".into(),
                group: "NIRUX Settings".into(),
                name: "Wi-Fi".into(),
                detail: "Connect, switch or turn the radio off".into(),
                keywords: "settings network wireless wifi ssid password".into(),
                favorite: false,
                target: mode("wifi"),
            },
        );
    }
    if in_path("niruwallpaper") {
        items.insert(
            1,
            Item {
                key: "settings:wallpaper".into(),
                group: "NIRUX Settings".into(),
                name: "Wallpaper".into(),
                detail: "Image, scale, rotation and screen".into(),
                keywords: "settings wallpaper background appearance".into(),
                favorite: false,
                target: Target::Spawn {
                    argv: vec!["niruwallpaper".to_string()],
                    terminal: false,
                    path: None,
                },
            },
        );
    }
    items
}

/// The effective idle, lock and display-off timers from `niructl timing`.
fn timing_items() -> Vec<Item> {
    let specs = [
        (
            "screensaver_timeout",
            "Screen saver after",
            "Idle minutes before the screen saver",
        ),
        (
            "lock_after_screensaver",
            "Lock after screen saver",
            "Minutes the screen saver stays up before locking",
        ),
        (
            "dpms_off_after_lock",
            "Display off after lock",
            "Minutes after locking before the display sleeps",
        ),
    ];
    let mut values: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    match Command::new("niructl").args(["timing", "show"]).output() {
        Ok(out) if out.status.success() => {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((key, value)) = line.split_once('=') {
                    values.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return vec![Item {
                key: "timing:unavailable".into(),
                group: "Idle and lock".into(),
                name: "Idle and lock timing unavailable".into(),
                detail: if text.is_empty() {
                    "niructl timing show failed".into()
                } else {
                    text
                },
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            }];
        }
        Err(e) => {
            return vec![Item {
                key: "timing:unavailable".into(),
                group: "Idle and lock".into(),
                name: "Idle and lock timing unavailable".into(),
                detail: format!("could not start niructl: {e}"),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            }];
        }
    }
    specs
        .into_iter()
        .map(|(key, label, detail)| {
            let value = values.get(key).cloned().unwrap_or_else(|| "?".into());
            Item {
                key: format!("timing:{key}"),
                group: "Idle and lock".into(),
                name: label.into(),
                detail: format!("{value} minutes · Enter to change"),
                keywords: format!("timing idle lock {detail}"),
                favorite: false,
                target: Target::Info,
            }
        })
        .collect()
}

fn input_items() -> Vec<Item> {
    let out = match Command::new("niructl")
        .args(["input", "status", "--json"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => {
            return vec![Item {
                key: "input:unavailable".into(),
                group: "Input".into(),
                name: "Input settings unavailable".into(),
                detail: "niructl input is missing; run nirux update".into(),
                keywords: "input keyboard mouse touchpad".into(),
                favorite: false,
                target: Target::Info,
            }];
        }
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let layout = value["keyboard"]["layout"].as_str().unwrap_or("");
    let delay = value["keyboard"]["repeat_delay"].as_u64().unwrap_or(0);
    let rate = value["keyboard"]["repeat_rate"].as_u64().unwrap_or(0);
    let natural = value["pointer"]["natural_scroll"].as_bool();
    let tap = value["pointer"]["tap"].as_bool();
    let speed = value["pointer"]["speed"].as_f64();
    let xinput = value["xinput_available"].as_bool().unwrap_or(false);
    let natural_supported = value["pointer_support"]["natural_scroll"]
        .as_bool()
        .unwrap_or(false);
    let tap_supported = value["pointer_support"]["tap"].as_bool().unwrap_or(false);
    let speed_supported = value["pointer_support"]["speed"].as_bool().unwrap_or(false);
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };
    let mut items = Vec::new();
    for candidate in ["se", "us", "gb", "de", "fr"] {
        items.push(Item {
            key: format!("input:layout:{candidate}"),
            group: "Input".into(),
            name: format!("Keyboard layout: {candidate}"),
            detail: if layout == candidate {
                "active".into()
            } else if layout.is_empty() {
                "system default is active".into()
            } else {
                "switch keyboard layout".into()
            },
            keywords: format!("input keyboard layout language {candidate}"),
            favorite: false,
            target: niructl(&["input", "set-keyboard", candidate]),
        });
    }
    for (name, d, r) in [("Slow", 500, 25), ("Normal", 250, 40), ("Fast", 150, 60)] {
        items.push(Item {
            key: format!("input:repeat:{d}:{r}"),
            group: "Input".into(),
            name: format!("Keyboard repeat: {name}"),
            detail: if delay == d && rate == r {
                "active".into()
            } else {
                format!("{d} ms / {r} per second")
            },
            keywords: format!("input keyboard repeat delay rate {name}"),
            favorite: false,
            target: niructl(&["input", "set-repeat", &d.to_string(), &r.to_string()]),
        });
    }
    if xinput && natural_supported {
        items.push(Item {
            key: "input:natural".into(),
            group: "Input".into(),
            name: format!(
                "Natural scrolling: {}",
                match natural {
                    Some(true) => "on",
                    Some(false) => "off",
                    None => "system",
                }
            ),
            detail: "Mouse wheel and touchpad scrolling direction".into(),
            keywords: "input pointer mouse touchpad natural scrolling".into(),
            favorite: false,
            target: niructl(&[
                "input",
                "set-pointer",
                "natural",
                if natural.unwrap_or(false) {
                    "off"
                } else {
                    "on"
                },
            ]),
        });
    }
    if xinput && tap_supported {
        items.push(Item {
            key: "input:tap".into(),
            group: "Input".into(),
            name: format!(
                "Touchpad tap-to-click: {}",
                match tap {
                    Some(true) => "on",
                    Some(false) => "off",
                    None => "system",
                }
            ),
            detail: "Tap the touchpad instead of pressing it".into(),
            keywords: "input pointer touchpad tap click".into(),
            favorite: false,
            target: niructl(&[
                "input",
                "set-pointer",
                "tap",
                if tap.unwrap_or(false) { "off" } else { "on" },
            ]),
        });
    }
    if xinput && speed_supported {
        items.push(Item {
            key: "input:speed".into(),
            group: "Input".into(),
            name: format!(
                "Pointer speed: {}",
                speed
                    .map(|current| format!("{current:.2}"))
                    .unwrap_or_else(|| "system".into())
            ),
            detail: "Click to cycle through -0.6 to 0.6".into(),
            keywords: "input pointer mouse touchpad speed acceleration".into(),
            favorite: false,
            target: niructl(&["input", "cycle-speed"]),
        });
    } else if !xinput {
        items.push(Item {
            key: "input:pointer-unavailable".into(),
            group: "Input".into(),
            name: "Pointer settings unavailable".into(),
            detail: "Install xinput with the next NIRUX update".into(),
            keywords: "input pointer mouse touchpad unavailable".into(),
            favorite: false,
            target: Target::Info,
        });
    } else if !natural_supported && !tap_supported && !speed_supported {
        items.push(Item {
            key: "input:pointer-driver".into(),
            group: "Input".into(),
            name: "Pointer driver does not expose libinput settings".into(),
            detail: "Keyboard settings still work; use the system input configuration".into(),
            keywords: "input pointer mouse touchpad driver unsupported".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    items.push(Item {
        key: "input:reset".into(),
        group: "Input".into(),
        name: "Reset input settings".into(),
        detail: "Remove NIRU overrides; the system defaults remain".into(),
        keywords: "input reset default keyboard pointer".into(),
        favorite: false,
        target: niructl(&["input", "reset"]),
    });
    items
}

fn sound_action_items() -> Vec<Item> {
    let mut items = Vec::new();
    for (key, name, detail, target) in [
        (
            "sound:open",
            "Open Sound…",
            "Output, input, applications and microphone test",
            Target::Mode("sound".into()),
        ),
        (
            "sound:mute-output",
            "Mute output",
            "Toggle the default output · niructl audio mute",
            spawn_niructl(&["audio", "mute"]),
        ),
        (
            "sound:mute-microphone",
            "Mute microphone",
            "Toggle input mute · niructl audio mic-mute toggle",
            spawn_niructl(&["audio", "mic-mute", "toggle"]),
        ),
        (
            "sound:mic-level-up",
            "Microphone level +",
            "Raise input gain without unmuting · niructl audio mic-level up",
            spawn_niructl(&["audio", "mic-level", "up"]),
        ),
        (
            "sound:mic-level-down",
            "Microphone level −",
            "Lower input gain without unmuting · niructl audio mic-level down",
            spawn_niructl(&["audio", "mic-level", "down"]),
        ),
        (
            "sound:mic-test",
            "Test microphone…",
            "Record five seconds locally and show peak/RMS",
            Target::SoundTest(5),
        ),
    ] {
        items.push(Item {
            key: key.into(),
            group: "Audio".into(),
            name: name.into(),
            detail: detail.into(),
            keywords: "sound audio volume mute microphone mic test level".into(),
            favorite: false,
            target,
        });
    }
    items
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
        ("Layout: Scroll", "Super+Ctrl+S", "layout scroll"),
        (
            "Scroll: one visible column",
            "Use in Scroll · niructl layout columns 1",
            "layout columns 1",
        ),
        (
            "Scroll: two visible columns",
            "Use in Scroll · niructl layout columns 2",
            "layout columns 2",
        ),
        (
            "Scroll: toggle visible columns",
            "Super+Ctrl+Shift+S",
            "layout columns toggle",
        ),
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
        let group = if matches!(
            first,
            "lock" | "screensaver" | "reload" | "dnd" | "idle" | "suspend"
        ) {
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
            keywords: String::new(),
            favorite: false,
            target: Target::Spawn {
                argv,
                terminal: false,
                path: None,
            },
        });
    }
    items.extend(sound_action_items());
    items.extend(theme_items());
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
                        keywords: String::new(),
                        favorite: false,
                        target: niructl(&["audio", kind.trim_end_matches('s'), id]),
                    });
                }
            }
        }
    }

    items.extend(shortcut_items());
    items.extend(settings_items());
    items.extend(input_items());
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
                    "doctor" => Target::Terminal {
                        argv: vec!["nirux".into(), "doctor".into()],
                        hold: true,
                    },
                    "audio" => Target::Terminal {
                        argv: vec!["niructl".into(), "audio".into(), "outputs".into()],
                        hold: true,
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
                    keywords: String::new(),
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
            keywords: String::new(),
            favorite: false,
            target: Target::Terminal {
                argv: vec!["nirux".into(), "update".into()],
                hold: true,
            },
        });
    }
    items.push(Item {
        key: "action:diagnostics".into(),
        group: "System".into(),
        name: "Run diagnostics".into(),
        detail: "Open the full NIRUX health report".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Terminal {
            argv: vec!["nirux".into(), "doctor".into()],
            hold: true,
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
                    keywords: String::new(),
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
            keywords: String::new(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["niruwallpaper".to_string()],
                terminal: false,
                path: None,
            },
        });
    }

    // The wallpaper window owns viewing and applying; NIRUX owns the verified
    // 255 MB download, so this row opens the installer in a terminal instead of
    // teaching the GUI a second download implementation.
    if in_path("nirux") {
        items.push(Item {
            key: "action:wallpaper:download-extras".into(),
            group: "Wallpaper".into(),
            name: "Download additional theme wallpapers…".into(),
            detail: "Carl Larsson and Erik Satie: red, blue, green and noir in 4K/8K".into(),
            keywords: "extra colour color wallpaper satie carl larsson red blue green noir".into(),
            favorite: false,
            target: Target::Terminal {
                argv: vec![
                    "nirux".to_string(),
                    "wallpapers".to_string(),
                    "download".to_string(),
                    "extras".to_string(),
                ],
                hold: true,
            },
        });
    }

    for ws in 1..=10 {
        items.push(Item {
            key: format!("action:workspace:{ws}"),
            group: "Workspaces".into(),
            name: format!("Workspace {ws}"),
            detail: "Switch the focused monitor".into(),
            keywords: String::new(),
            favorite: false,
            target: niructl(&["tag", &ws.to_string()]),
        });
    }
    items.push(Item {
        key: "action:nirux-update".into(),
        group: "System".into(),
        name: "Update NIRUX suite…".into(),
        detail: "Run nirux update in a terminal".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Terminal {
            argv: vec!["nirux".into(), "update".into()],
            hold: true,
        },
    });
    items.push(Item {
        key: "action:reset-bar".into(),
        group: "Appearance".into(),
        name: "Reset bar configuration…".into(),
        detail: "Restore current NIRUBAR defaults; a backup is kept".into(),
        keywords: String::new(),
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
            keywords: String::new(),
            favorite: false,
            target: Target::Confirm(action),
        });
    }
    // NIRU Actions: practical actions that need no agent, network or model.
    for (key, name, detail, argv) in [
        (
            "maintenance:storage-review",
            "Review storage…",
            "Inspect caches, trash, old downloads and NIRUX generations; nothing is removed",
            vec!["niructl", "storage", "review"],
        ),
        (
            "maintenance:windows-rescue",
            "Bring off-screen windows back",
            "Move managed windows that are outside every connected display",
            vec!["niructl", "windows", "rescue"],
        ),
        (
            "maintenance:tasks",
            "Recent NIRU tasks",
            "Show deterministic and OpenCode-assisted task results",
            vec!["niructl", "task", "list"],
        ),
        (
            "maintenance:health",
            "NIRUX installed and running",
            "Show the installed generation and current desktop state",
            vec!["nirux", "status"],
        ),
    ] {
        if argv.first() == Some(&"nirux") && !in_path("nirux") {
            continue;
        }
        items.push(Item {
            key: format!("action:{key}"),
            group: "NIRU Actions".into(),
            name: name.into(),
            detail: detail.into(),
            keywords: format!("system action {key}").replace(':', " "),
            favorite: false,
            target: Target::Terminal {
                argv: argv.into_iter().map(str::to_string).collect(),
                hold: true,
            },
        });
    }
    for (key, name, action) in [
        ("focus-60", "Focus mode for 60 minutes", "focus"),
        (
            "presentation-60",
            "Presentation mode for 60 minutes",
            "presentation",
        ),
    ] {
        items.push(Item {
            key: format!("action:mode:{key}"),
            group: "Work contexts".into(),
            name: name.into(),
            detail: "DND and keep-awake; restored when the timer ends".into(),
            keywords: format!("context mode {action} dnd idle meeting presentation focus"),
            favorite: false,
            target: niructl(&["mode", action, "60"]),
        });
    }
    items.push(Item {
        key: "action:mode:stop".into(),
        group: "Work contexts".into(),
        name: "Stop work context".into(),
        detail: "Restore DND and idle inhibition changed by the active mode".into(),
        keywords: "context mode stop restore dnd idle".into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["niructl".into(), "mode".into(), "stop".into()],
            terminal: false,
            path: None,
        },
    });
    if in_path("opencode") {
        items.push(Item {
            key: "action:agent:opencode".into(),
            group: "NIRU Actions".into(),
            name: "Ask OpenCode…".into(),
            detail: "Open a terminal task handoff with local NIRUX diagnostics; ask before changes"
                .into(),
            keywords: "agent opencode ai assist task diagnose".into(),
            favorite: false,
            target: Target::Terminal {
                argv: vec!["niructl".into(), "agent".into()],
                hold: false,
            },
        });
    } else {
        items.push(Item {
            key: "action:agent:unavailable".into(),
            group: "NIRU Actions".into(),
            name: "OpenCode assistance unavailable".into(),
            detail: "OpenCode is not installed; the deterministic actions still work".into(),
            keywords: "agent opencode ai unavailable".into(),
            favorite: false,
            target: Target::Info,
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
            keywords: app.keywords.join(" "),
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
            keywords: String::new(),
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

/// True when every character of `needle` appears in `haystack` in order.
fn subsequence(haystack: &str, needle: &str) -> bool {
    let mut wanted = needle.chars();
    let Some(mut next) = wanted.next() else {
        return true;
    };
    for ch in haystack.chars() {
        if ch == next {
            match wanted.next() {
                Some(ch) => next = ch,
                None => return true,
            }
        }
    }
    false
}

/// How strongly one word matches an item; lower is better.
///
/// The order is deliberate: exact and prefix matches stay in front of a
/// multiword hit, metadata comes after the visible name, and fuzzy matching is
/// last so a typo never hides an obvious result.
fn term_rank(
    term: &str,
    name: &str,
    label: &str,
    group: &str,
    detail: &str,
    keywords: &str,
) -> Option<u8> {
    if name == term {
        Some(0)
    } else if name.starts_with(term) || label.starts_with(term) {
        Some(1)
    } else if name
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word == term)
    {
        Some(2)
    } else if name.contains(term) || label.contains(term) {
        Some(3)
    } else if keywords.contains(term) || group.contains(term) || detail.contains(term) {
        Some(4)
    } else if term.chars().count() >= 3 && (subsequence(name, term) || subsequence(label, term)) {
        Some(5)
    } else {
        None
    }
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
    let terms: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(bool, u16, usize, String, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let label = item.label().to_lowercase();
            let name = item.name.to_lowercase();
            let group = item.group.to_lowercase();
            let detail = item.detail.to_lowercase();
            let keywords = item.keywords.to_lowercase();
            let rank = if terms.is_empty() {
                0
            } else {
                let mut total = 0u16;
                for term in &terms {
                    total += u16::from(term_rank(term, &name, &label, &group, &detail, &keywords)?);
                }
                total
            };
            // A shorter label is a closer match only while searching; with an
            // empty field the list stays alphabetical.
            let length = if terms.is_empty() { 0 } else { label.len() };
            Some((!pinned.contains(&i), rank, length, label.clone(), i))
        })
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
    });
    hits.into_iter().map(|(_, _, _, _, i)| i).collect()
}

fn system_action_priority(group: &str) -> u8 {
    match group {
        "NIRU Actions" => 0,
        "Work contexts" => 1,
        "Saved actions" => 2,
        "Diagnostics" => 3,
        "System" => 4,
        "Audio" => 5,
        "Display" => 6,
        "Layouts" => 7,
        "Appearance" => 8,
        "Settings" | "NIRUX Settings" => 9,
        "Recipes" => 10,
        "Power" => 11,
        "Shortcuts" => 12,
        "Themes" => 13,
        "Wallpaper" => 14,
        "Workspaces" => 15,
        _ => 16,
    }
}

fn filtered_for_mode(
    items: &[Item],
    query: &str,
    usage: &std::collections::HashMap<String, u64>,
    mode: &str,
) -> Vec<usize> {
    if mode == "system-actions" && query.trim().is_empty() {
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|index| {
            (
                system_action_priority(&items[*index].group),
                items[*index].label().to_lowercase(),
            )
        });
        order
    } else {
        filtered(items, query, usage)
    }
}

/// Place the intent rows (appended after the plain rows) before every ordinary
/// search result.
fn intent_first_order(base_len: usize, intent_count: usize, base_order: Vec<usize>) -> Vec<usize> {
    if intent_count == 0 {
        return base_order;
    }
    (base_len..base_len + intent_count)
        .chain(base_order)
        .collect()
}

/// The complete row list for the dedicated web-search mode.
///
/// Kept outside `Ui` so the empty, one-character, alias and address cases can
/// be tested without opening an X connection.
fn web_rows(query: &str, catalog: &websearch::Catalog) -> Vec<Item> {
    let trimmed = query.trim();
    if catalog.sources.is_empty() {
        return vec![Item {
            key: "search:none".into(),
            group: "Search".into(),
            name: "No search engines are configured".into(),
            detail: "Add [[search.sources]] to ~/.config/nirulauncher/config.toml".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }];
    }
    if trimmed.is_empty() {
        return catalog
            .sources
            .iter()
            .map(websearch::provider_item)
            .collect();
    }
    // Keep a trailing space so `gh ` is an empty prefixed query and does not
    // fall back to searching for the word "gh".
    if let Some((source, rest)) = catalog.prefixed(query.trim_start()) {
        if rest.is_empty() {
            return vec![websearch::provider_item(source)];
        }
        return websearch::item(source, &rest).into_iter().collect();
    }
    let mut rows = Vec::new();
    if let Some(url) = websearch::web_address(trimmed) {
        rows.push(websearch::address_item(&url));
    }
    for source in &catalog.sources {
        if let Some(item) = websearch::item(source, trimmed) {
            rows.push(item);
        }
    }
    rows
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

impl Drop for Ui {
    fn drop(&mut self) {
        if let Some(test) = &self.sound_test {
            sound::cleanup(&test.file);
        }
    }
}

impl Ui {
    fn new(mode: &str) -> Result<Self> {
        let cfg = Config::load()?;
        let apps = desktop::discover();
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        let saved_actions = if matches!(mode, "actions" | "system-actions" | "apps") {
            load_saved_actions()
        } else {
            Vec::new()
        };
        let mut items = Vec::new();
        if mode == "actions" {
            items.extend(action_items(&cfg));
            append_saved_actions(&mut items, &saved_actions);
            items.extend(window_items(&c, root));
            items.extend(app_items(apps));
        } else if mode == "system-actions" {
            items.extend(action_items(&cfg));
            append_saved_actions(&mut items, &saved_actions);
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
            // Dedicated web search: the provider rows are the whole list and
            // are built from the query, including when it is empty.
        } else if mode == "keys" {
            items.extend(shortcut_items());
        } else if mode == "settings" {
            items.extend(settings_items());
        } else if mode == "themes" {
            items.extend(theme_items());
        } else if mode == "timing" {
            items.extend(timing_items());
        } else if mode == "input" {
            items.extend(input_items());
        } else if mode == "sound" {
            items.extend(sound_items(None, None, Some("Loading audio…")));
        } else {
            items.extend(app_items(apps));
            append_saved_actions(&mut items, &saved_actions);
        }
        let runtime_favorites = load_runtime_favorites();
        let removed_favorites = load_removed_favorites();
        for item in &mut items {
            let configured = cfg.favorites.iter().any(|favorite| {
                favorite == &item.key
                    || favorite.eq_ignore_ascii_case(&item.name)
                    || favorite == &item.label()
            });
            item.favorite = (item.favorite
                || configured
                || runtime_favorites.iter().any(|key| key == &item.key))
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
        if items.is_empty() && mode != "search" && mode != "wifi" && mode != "sound" {
            return Err("no launchable applications or actions found".into());
        }
        let (px, py) = point(&c, root);
        let monitor = monitor_at(&c, root, px, py);
        let web_mode = mode == "search";
        let catalog = websearch::catalog(
            &cfg.search_sources,
            &cfg.search_order,
            cfg.search_default.as_deref(),
            &cfg.search_aliases,
        );
        // The provider rows are built from the query, so the window is opened
        // tall enough for all of them instead of resizing later.
        let search_slot = if web_mode {
            catalog.sources.len().saturating_add(1)
        } else {
            0
        };
        // Settings opens taller than its own short list so the views it
        // switches to (Keybindings, Idle timing) are not shown in a five-row
        // window; the launcher never resizes while it is open.
        let rows = if matches!(mode, "settings" | "wifi" | "system-actions" | "sound") {
            cfg.max_rows
        } else {
            (items.len() + search_slot).min(cfg.max_rows).max(1)
        };
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
        let filtered = filtered_for_mode(&items, "", &usage, mode);
        let mut ui = Self {
            c,
            root,
            win,
            gc,
            cfg,
            renderer,
            plain_items: items.len(),
            profile_name: None,
            number_edit: None,
            wifi: None,
            wifi_rx: None,
            wifi_busy: None,
            wifi_secret: None,
            sound: None,
            sound_rx: None,
            sound_test_rx: None,
            sound_test: None,
            sound_busy: None,
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
            mode: mode.to_string(),
            catalog,
        };
        ui.refilter();
        if mode == "wifi" {
            ui.enter_wifi()?;
        } else if mode == "sound" {
            ui.enter_sound()?;
        }
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
        let title = if self.wifi_secret.is_some() {
            "Connect to Wi-Fi"
        } else if self.number_edit.is_some() {
            "Idle and lock timing"
        } else if let Some(name) = &self.profile_name {
            let _ = name;
            "Name the layout profile"
        } else if self.confirm.is_some() {
            "Confirm"
        } else if self.web_mode {
            "Search the web"
        } else if self.items.iter().all(|item| item.group == "Shortcuts") {
            "Keybindings"
        } else if self.items.iter().all(|item| item.group == "NIRUX Settings") {
            "NIRUX Settings"
        } else if self.items.iter().all(|item| item.group == "Input") {
            "Input devices"
        } else if self.items.iter().all(|item| item.group == "Appearance") {
            "Themes"
        } else if self
            .items
            .iter()
            .any(|item| item.key.starts_with("timing:"))
        {
            "Idle and lock timing"
        } else if self.items.iter().all(|item| item.group == "Audio") {
            "Microphone controls"
        } else if self.items.iter().all(|item| item.group == "Displays") {
            "Displays"
        } else if self.items.iter().all(|item| item.group == "Bluetooth") {
            "Bluetooth"
        } else if self.mode == "wifi" {
            "Wi-Fi"
        } else if self.mode == "sound" {
            "Sound"
        } else if self.mode == "system-actions" {
            "NIRU Actions"
        } else if self.items.iter().all(|item| item.group == "Power") {
            "Power"
        } else if self
            .items
            .iter()
            .any(|i| matches!(i.target, Target::Confirm(_)))
        {
            "Search apps and actions"
        } else {
            "Search applications"
        };
        let query = if let Some(secret) = &self.wifi_secret {
            if secret.hidden && secret.stage == 0 {
                format!("Hidden network SSID: {}", secret.buffer)
            } else {
                format!(
                    "Password for {}: {}",
                    secret.ssid,
                    "*".repeat(secret.buffer.chars().count())
                )
            }
        } else if let Some(busy) = &self.wifi_busy {
            format!("{title}: {busy}")
        } else if let Some(edit) = &self.number_edit {
            format!("{}: {}", edit.label, edit.buffer)
        } else if let Some(name) = &self.profile_name {
            format!("{title}: {name}")
        } else if self.query.is_empty() {
            title.to_string()
        } else {
            format!("{title}: {}", self.query)
        };
        let colour = if self.query.is_empty()
            && self.number_edit.is_none()
            && self.wifi_secret.is_none()
            && self.wifi_busy.is_none()
        {
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
                    Action::ForgetWifi(_) => "forget this network",
                }
            )
        } else if self.number_edit.is_some() {
            "Type minutes · Enter saves · Esc cancels".to_string()
        } else if self.profile_name.is_some() {
            "Enter saves the layout · Esc cancels".to_string()
        } else if !self.error.is_empty() {
            self.error.clone()
        } else if let Some(secret) = &self.wifi_secret {
            if secret.hidden && secret.stage == 0 {
                "Type the hidden SSID · Enter continues · Esc cancels".to_string()
            } else {
                "Type the password · Enter connects · Esc cancels".to_string()
            }
        } else if let Some(busy) = &self.wifi_busy {
            busy.clone()
        } else if self.mode == "wifi" {
            "Enter connects or acts · Ctrl+R rescan · Esc close".to_string()
        } else if self.mode == "sound" {
            "Enter selects or acts · Esc close · ↑↓ move".to_string()
        } else if self.web_mode {
            "Enter opens search · Tab moves · Esc closes".to_string()
        } else if self.mode == "keys" {
            "Esc close · ↑↓ move · type to search".to_string()
        } else if self.mode == "settings" {
            "Enter opens · Esc close · ↑↓ move".to_string()
        } else if self.mode == "themes" {
            "Enter applies · Esc close · ↑↓ move".to_string()
        } else if self
            .items
            .iter()
            .any(|item| item.key.starts_with("timing:"))
        {
            "Enter changes a value · Esc close · ↑↓ move".to_string()
        } else {
            "Enter launch · Esc close · ↑↓ move · Ctrl+P favorite · Ctrl+S save action".to_string()
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

    /// Open the compact sound panel. It paints immediately, then loads the
    /// PipeWire snapshot in a worker so the launcher never waits on wpctl.
    fn enter_sound(&mut self) -> Result<()> {
        self.mode = "sound".into();
        self.query.clear();
        self.confirm = None;
        self.number_edit = None;
        self.profile_name = None;
        self.error.clear();
        if let Some(old) = self.sound_test.take() {
            sound::cleanup(&old.file);
        }
        self.sound = None;
        self.refresh_sound_items();
        self.start_sound_job("Loading audio…".into())
    }

    fn refresh_sound_items(&mut self) {
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        let snapshot = self.sound.clone();
        let test = self.sound_test.clone();
        let busy = self.sound_busy.clone();
        self.items = sound_items(snapshot.as_ref(), test.as_ref(), busy.as_deref());
        self.plain_items = self.items.len();
        self.filtered = filtered_for_mode(&self.items, &self.query, &self.usage, &self.mode);
        self.selected = previous
            .and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&index| self.items[index].key == key)
            })
            .unwrap_or(0)
            .min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
    }

    fn start_sound_job(&mut self, label: String) -> Result<()> {
        if self.sound_rx.is_some() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        self.sound_rx = Some(rx);
        self.error.clear();
        self.sound_busy = Some(label);
        thread::spawn(move || {
            let _ = tx.send(sound::status());
        });
        self.draw()
    }

    fn poll_sound(&mut self) -> Result<bool> {
        let Some(rx) = self.sound_rx.as_ref() else {
            return Ok(false);
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => Err("the audio worker stopped".into()),
        };
        self.sound_rx = None;
        self.sound_busy = None;
        match result {
            Ok(snapshot) => self.sound = Some(snapshot),
            Err(error) => self.error = error,
        }
        self.refresh_sound_items();
        self.draw()?;
        Ok(true)
    }

    fn start_sound_test(&mut self, seconds: u64) -> Result<()> {
        if self.sound_test_rx.is_some() {
            self.error = "Microphone test is already running".into();
            self.draw()?;
            return Ok(());
        }
        if let Some(old) = self.sound_test.take() {
            sound::cleanup(&old.file);
        }
        let (tx, rx) = mpsc::channel();
        self.sound_test_rx = Some(rx);
        self.mode = "sound".into();
        self.sound_busy = Some(format!("Testing microphone for {seconds} seconds…"));
        self.error.clear();
        self.refresh_sound_items();
        self.draw()?;
        thread::spawn(move || {
            let _ = tx.send(sound::microphone_test(seconds));
        });
        Ok(())
    }

    fn poll_sound_test(&mut self) -> Result<bool> {
        let Some(rx) = self.sound_test_rx.as_ref() else {
            return Ok(false);
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("the microphone test worker stopped".into())
            }
        };
        self.sound_test_rx = None;
        self.sound_busy = None;
        match result {
            Ok(test) => self.sound_test = Some(test),
            Err(error) => self.error = error,
        }
        self.refresh_sound_items();
        self.draw()?;
        Ok(true)
    }

    /// Open the Wi-Fi view with a quick status, then scan in a worker.
    fn enter_wifi(&mut self) -> Result<()> {
        self.mode = "wifi".into();
        self.query.clear();
        self.confirm = None;
        self.number_edit = None;
        self.profile_name = None;
        self.wifi_secret = None;
        self.error.clear();
        // Mapping the window must not wait for D-Bus or privilege prompts.
        self.wifi = Some(wifi::Snapshot {
            present: wifi::hardware_present(),
            powered: true,
            ..Default::default()
        });
        self.refresh_wifi_items();
        if let Some(index) = self
            .filtered
            .iter()
            .position(|&index| self.items[index].key == "wifi:connected")
        {
            self.selected = index;
        }
        self.start_wifi_job("Scanning for networks…".into(), || Ok(()))
    }

    fn refresh_wifi_items(&mut self) {
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        let Some(snapshot) = &self.wifi else {
            return;
        };
        self.items = wifi_items(snapshot);
        self.plain_items = self.items.len();
        self.filtered = wifi_filtered(&self.items, &self.query);
        self.selected = previous
            .and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&index| self.items[index].key == key)
            })
            .unwrap_or(0)
            .min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
    }

    fn start_wifi_job<F>(&mut self, label: String, job: F) -> Result<()>
    where
        F: FnOnce() -> std::result::Result<(), String> + Send + 'static,
    {
        if self.wifi_rx.is_some() {
            self.error = "Wait for the current Wi-Fi operation".into();
            self.draw()?;
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        self.wifi_rx = Some(rx);
        self.wifi_busy = Some(label);
        self.error.clear();
        thread::spawn(move || {
            let result = job().map(|_| wifi::snapshot(true));
            let _ = tx.send(result);
        });
        self.draw()
    }

    fn poll_wifi(&mut self) -> Result<bool> {
        let Some(rx) = self.wifi_rx.as_ref() else {
            return Ok(false);
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The Wi-Fi worker stopped unexpectedly".into())
            }
        };
        self.wifi_rx = None;
        self.wifi_busy = None;
        match result {
            Ok(snapshot) => {
                let first_scan = self.wifi.as_ref().is_some_and(|old| old.manager.is_empty());
                self.error = snapshot.error.clone();
                self.wifi = Some(snapshot);
                self.refresh_wifi_items();
                if first_scan && self.query.trim().is_empty() {
                    self.selected = self
                        .filtered
                        .iter()
                        .position(|&index| self.items[index].key == "wifi:connected")
                        .unwrap_or(0);
                }
            }
            Err(error) => self.error = error,
        }
        self.draw()?;
        Ok(true)
    }

    /// Handle a Wi-Fi row. `None` means this was not a Wi-Fi action.
    fn wifi_execute(&mut self, key: &str) -> Result<Option<bool>> {
        if let Some(id) = key.strip_prefix("wifi:profile:") {
            let id = id.to_owned();
            self.start_wifi_job("Activating saved profile…".into(), move || {
                wifi::connect_profile(&id)
            })?;
            return Ok(Some(true));
        }
        match key {
            "wifi:refresh" => {
                self.start_wifi_job("Scanning for networks…".into(), || Ok(()))?;
                return Ok(Some(true));
            }
            "wifi:power" => {
                let on = self.wifi.as_ref().map(|s| !s.powered).unwrap_or(true);
                let label = if on {
                    "Turning Wi-Fi on…"
                } else {
                    "Turning Wi-Fi off…"
                };
                self.start_wifi_job(label.into(), move || wifi::set_power(on))?;
                return Ok(Some(true));
            }
            "wifi:disconnect" => {
                self.start_wifi_job("Disconnecting…".into(), wifi::disconnect)?;
                return Ok(Some(true));
            }
            "wifi:connected" => return Ok(Some(true)),
            "wifi:hidden-wpa" | "wifi:hidden-open" => {
                self.wifi_secret = Some(WifiSecret {
                    ssid: String::new(),
                    security: if key == "wifi:hidden-open" {
                        "Open".into()
                    } else {
                        "WPA2/WPA3".into()
                    },
                    buffer: String::new(),
                    hidden: true,
                    stage: 0,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
            _ => {}
        }
        if let Some(ssid) = key.strip_prefix("wifi:forget:") {
            self.confirm = Some(Action::ForgetWifi(ssid.to_string()));
            self.draw()?;
            return Ok(Some(true));
        }
        if let Some(ssid) = key.strip_prefix("wifi:connect-pw:") {
            if let Some(network) = self
                .wifi
                .as_ref()
                .and_then(|snapshot| snapshot.networks.iter().find(|n| n.ssid == ssid))
            {
                self.wifi_secret = Some(WifiSecret {
                    ssid: network.ssid.clone(),
                    security: network.security.clone(),
                    buffer: String::new(),
                    hidden: false,
                    stage: 1,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
        }
        if let Some(ssid) = key.strip_prefix("wifi:connect:") {
            let Some(network) = self
                .wifi
                .as_ref()
                .and_then(|snapshot| snapshot.networks.iter().find(|n| n.ssid == ssid))
                .cloned()
            else {
                return Ok(Some(true));
            };
            if !network.can_connect() {
                self.error = "this network needs a saved enterprise profile".into();
                self.draw()?;
                return Ok(Some(true));
            }
            if network.saved && !network.security.contains("Enterprise") {
                let ssid = network.ssid.clone();
                self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                    wifi::connect(&ssid, "Saved", "", false)
                })?;
                return Ok(Some(true));
            }
            if network.needs_secret() {
                self.wifi_secret = Some(WifiSecret {
                    ssid: network.ssid.clone(),
                    security: network.security.clone(),
                    buffer: String::new(),
                    hidden: false,
                    stage: 1,
                });
                self.error.clear();
                self.draw()?;
                return Ok(Some(true));
            }
            let ssid = network.ssid.clone();
            let security = network.security.clone();
            self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                wifi::connect(&ssid, &security, "", false)
            })?;
            return Ok(Some(true));
        }
        Ok(None)
    }

    fn wifi_secret_key(&mut self, key: u32) -> Result<()> {
        let Some(mut secret) = self.wifi_secret.take() else {
            return Ok(());
        };
        match key {
            0xff1b => {
                self.error.clear();
            }
            0xff0d | 0xff8d => {
                if secret.hidden && secret.stage == 0 {
                    let ssid = secret.buffer.trim().to_string();
                    if ssid.is_empty() {
                        self.error = "Enter the hidden network name".into();
                        self.wifi_secret = Some(secret);
                    } else {
                        secret.ssid = ssid;
                        secret.stage = 1;
                        secret.buffer.clear();
                        if secret.security == "Open" {
                            let ssid = secret.ssid.clone();
                            self.wifi_secret = None;
                            self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                                wifi::connect(&ssid, "Open", "", true)
                            })?;
                            return Ok(());
                        }
                        self.wifi_secret = Some(secret);
                    }
                } else {
                    let ssid = secret.ssid.clone();
                    let security = secret.security.clone();
                    let password = secret.buffer.clone();
                    self.wifi_secret = None;
                    self.start_wifi_job(format!("Connecting to {ssid}…"), move || {
                        wifi::connect(&ssid, &security, &password, secret.hidden)
                    })?;
                    return Ok(());
                }
            }
            0xff08 => {
                secret.buffer.pop();
                self.wifi_secret = Some(secret);
            }
            _ if is_text_key(key) => {
                if let Some(ch) = char::from_u32(key) {
                    if !ch.is_control() && (secret.hidden && secret.stage == 0 || ch != '\0') {
                        secret.buffer.push(ch);
                    }
                }
                self.wifi_secret = Some(secret);
            }
            _ => self.wifi_secret = Some(secret),
        }
        self.draw()
    }

    /// Save the selected command row as a persistent action. The launcher only
    /// writes through `niructl action save`; it never edits actions.toml.
    fn save_selected_action(&mut self) -> Result<()> {
        let Some(&index) = self.filtered.get(self.selected) else {
            self.error = "Select an app or command row to save".into();
            self.draw()?;
            return Ok(());
        };
        let name = self.items[index].name.clone();
        let argv = match savable_argv(&self.items[index].target) {
            Ok(argv) => argv.to_vec(),
            Err(message) => {
                self.error = message.into();
                self.draw()?;
                return Ok(());
            }
        };
        let existing = match load_saved_actions_result() {
            Ok(existing) => existing,
            Err(reason) => {
                self.error = format!("Saving actions is unavailable: {reason}");
                self.draw()?;
                return Ok(());
            }
        };
        let saved_name = saved::unique_name(&name, existing.iter().map(|item| item.name.as_str()));
        let output = Command::new("niructl")
            .args(["action", "save", &saved_name, "--"])
            .args(&argv)
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(out) if out.status.success() => {
                self.error = format!("Saved action: {saved_name}");
            }
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let lower = text.to_lowercase();
                self.error = if lower.contains("unknown command")
                    || lower.contains("unknown subcommand")
                    || lower.contains("unrecognized subcommand")
                    || lower.contains("invalid subcommand")
                {
                    "Saving actions is unavailable: this niructl has no action command".into()
                } else if text.is_empty() {
                    format!("Could not save action: {saved_name}")
                } else {
                    format!("Could not save action: {text}")
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.error = "Saving actions is unavailable: niructl was not found".into();
            }
            Err(error) => {
                self.error = format!("Could not start niructl: {error}");
            }
        }
        self.draw()?;
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

    /// Keys while a layout profile is being named.
    fn name_key(&mut self, key: u32) -> Result<()> {
        let Some(mut name) = self.profile_name.take() else {
            return Ok(());
        };
        match key {
            0xff1b => {
                self.error.clear();
            }
            0xff0d | 0xff8d => {
                let trimmed = name.trim().to_string();
                if trimmed.is_empty() {
                    self.error = "A profile needs a name".into();
                    self.profile_name = Some(name);
                } else {
                    match Command::new("niructl")
                        .args(["display", "save", &trimmed])
                        .output()
                    {
                        Ok(out) if out.status.success() => {
                            self.error = format!("Saved the layout as {trimmed}");
                        }
                        Ok(out) => {
                            let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                            self.error = if text.is_empty() {
                                format!("Could not save the layout as {trimmed}")
                            } else {
                                text
                            };
                        }
                        Err(e) => self.error = format!("could not start niructl: {e}"),
                    }
                }
            }
            0xff08 => {
                name.pop();
                self.profile_name = Some(name);
            }
            _ if is_text_key(key) => {
                if let Some(ch) = char::from_u32(key) {
                    if !ch.is_control() && (!ch.is_whitespace() || !name.ends_with(' ')) {
                        name.push(ch);
                    }
                }
                self.profile_name = Some(name);
            }
            _ => self.profile_name = Some(name),
        }
        self.draw()
    }

    /// Keys while an idle/lock timing value is being edited.
    fn number_key(&mut self, key: u32) -> Result<()> {
        let Some(mut edit) = self.number_edit.take() else {
            return Ok(());
        };
        match key {
            0xff1b => {
                self.error.clear();
            }
            0xff0d | 0xff8d => {
                let value = edit.buffer.trim().parse::<u32>();
                match value {
                    Ok(minutes) if minutes <= 24 * 60 => {
                        let minutes_text = minutes.to_string();
                        match Command::new("niructl")
                            .args(["timing", "set", &edit.key, &minutes_text])
                            .output()
                        {
                            Ok(out) if out.status.success() => {
                                let key = format!("timing:{}", edit.key);
                                if let Some(item) =
                                    self.items.iter_mut().find(|item| item.key == key)
                                {
                                    item.detail = format!("{minutes} minutes · Enter to change");
                                }
                                self.error = format!("{} saved", edit.label);
                            }
                            Ok(out) => {
                                let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                self.error = if text.is_empty() {
                                    "Could not save the timing".into()
                                } else {
                                    text
                                };
                                self.number_edit = Some(edit);
                            }
                            Err(e) => {
                                self.error = format!("could not start niructl: {e}");
                                self.number_edit = Some(edit);
                            }
                        }
                    }
                    _ => {
                        self.error = "Enter a whole number of minutes (0-1440)".into();
                        self.number_edit = Some(edit);
                    }
                }
            }
            0xff08 => {
                edit.buffer.pop();
                self.number_edit = Some(edit);
            }
            _ if (0x30..=0x39).contains(&key) => {
                if edit.buffer.len() < 4 {
                    if let Some(ch) = char::from_u32(key) {
                        edit.buffer.push(ch);
                    }
                }
                self.number_edit = Some(edit);
            }
            _ => self.number_edit = Some(edit),
        }
        self.draw()
    }

    /// The rows for the dedicated web-search mode.
    fn web_items(&self) -> Vec<Item> {
        web_rows(&self.query, &self.catalog)
    }

    fn refilter(&mut self) {
        self.confirm = None;
        self.error.clear();
        if self.mode == "wifi" {
            self.refresh_wifi_items();
            return;
        }
        if self.web_mode {
            let previous = self
                .filtered
                .get(self.selected)
                .map(|&index| self.items[index].key.clone());
            self.items = self.web_items();
            self.filtered = (0..self.items.len()).collect();
            let remembered =
                previous.and_then(|key| self.items.iter().position(|item| item.key == key));
            let index = remembered.unwrap_or_else(|| {
                if self.query.trim().is_empty() {
                    self.catalog.default_index()
                } else {
                    0
                }
            });
            self.selected = index.min(self.items.len().saturating_sub(1));
            self.offset = 0;
            if self.catalog.sources.is_empty() {
                self.error = "No search engine is configured; see the launcher README".to_string();
            } else if let Some((_, rest)) = self.catalog.prefixed(self.query.trim_start()) {
                if rest.is_empty() {
                    self.error = "Type a search phrase after the provider alias".to_string();
                }
            }
            return;
        }
        self.items.truncate(self.plain_items);
        let intent_rows = if matches!(self.mode.as_str(), "apps" | "actions" | "system-actions")
            && !self.query.trim().is_empty()
        {
            intent_items(&self.query)
        } else {
            Vec::new()
        };
        let base_order = if self.query.trim().is_empty()
            && matches!(self.mode.as_str(), "settings" | "timing" | "wifi")
        {
            // These views have a deliberate order: appearance first, then the
            // lock ladder. Alphabetical sorting would scramble both.
            (0..self.items.len()).collect()
        } else {
            filtered_for_mode(&self.items, &self.query, &self.usage, &self.mode)
        };
        let first = self.items.len();
        let count = intent_rows.len();
        self.items.extend(intent_rows);
        self.filtered = intent_first_order(first, count, base_order);
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
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

    /// The rows a settings sub-view shows. These are built in-process so
    /// choosing one swaps the open launcher instead of racing a second one.
    fn mode_items(&self, mode: &str) -> Vec<Item> {
        match mode {
            "wifi" => self
                .wifi
                .as_ref()
                .map(wifi_items)
                .unwrap_or_else(|| wifi_items(&wifi::snapshot(false))),
            "keys" => shortcut_items(),
            "settings" => settings_items(),
            "themes" => theme_items(),
            "timing" => timing_items(),
            "input" => input_items(),
            "sound" => sound_items(
                self.sound.as_ref(),
                self.sound_test.as_ref(),
                self.sound_busy.as_deref(),
            ),
            "microphone" => microphone_items(),
            "bluetooth" => bluetooth_items(),
            "displays" => display_items(),
            "power" => power_items(),
            "windows" => window_items(&self.c, self.root),
            _ => Vec::new(),
        }
    }

    fn switch_mode(&mut self, mode: &str) -> Result<()> {
        if mode == "wifi" {
            return self.enter_wifi();
        }
        if mode == "sound" {
            return self.enter_sound();
        }
        let items = self.mode_items(mode);
        if items.is_empty() {
            self.error = "Nothing to show in this view".into();
            self.draw()?;
            return Ok(());
        }
        self.items = items;
        self.plain_items = self.items.len();
        self.web_mode = false;
        self.mode = mode.to_string();
        self.query.clear();
        self.number_edit = None;
        self.profile_name = None;
        self.confirm = None;
        self.filtered = filtered_for_mode(&self.items, "", &self.usage, &self.mode);
        self.selected = 0;
        self.offset = 0;
        self.error.clear();
        self.draw()
    }

    /// A link, opened with the same code path as every other spawn.
    fn execute(&mut self, key: &str, target: Target) -> Result<bool> {
        if key == "display:save" {
            self.profile_name = Some(String::new());
            self.error.clear();
            self.draw()?;
            return Ok(true);
        }
        if self.mode == "wifi" {
            if let Some(keep) = self.wifi_execute(key)? {
                return Ok(keep);
            }
        }
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
                        Action::ForgetWifi(ssid) => {
                            self.confirm = None;
                            self.start_wifi_job(format!("Forgetting {ssid}…"), move || {
                                wifi::forget(&ssid)
                            })?;
                            return Ok(true);
                        }
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
            Target::Mode(mode) => {
                self.switch_mode(&mode)?;
                Ok(true)
            }
            Target::SoundTest(seconds) => {
                self.start_sound_test(seconds)?;
                Ok(true)
            }
            Target::Info => Ok(true),
            Target::Terminal { argv, hold } => {
                match spawn_terminal_command(&argv, hold, &self.cfg) {
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
                }
            }
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
        if self.web_mode {
            let trimmed = self.query.trim();
            if trimmed.is_empty() {
                self.error = "Type a search phrase".into();
                self.draw()?;
                return Ok(true);
            }
            if let Some((_, rest)) = self.catalog.prefixed(self.query.trim_start()) {
                if rest.is_empty() {
                    self.error = "Type a search phrase after the provider alias".into();
                    self.draw()?;
                    return Ok(true);
                }
            }
        }
        let Some(index) = self.filtered.get(self.selected).copied() else {
            return Ok(true);
        };
        let key = self.items[index].key.clone();
        if let Some(name) = key.strip_prefix("timing:") {
            self.number_edit = Some(NumberEdit {
                key: name.to_string(),
                label: self.items[index].name.clone(),
                buffer: String::new(),
            });
            self.error.clear();
            self.draw()?;
            return Ok(true);
        }
        let target = self.items[index].target.clone();
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
            if self.wifi_rx.is_some() && self.poll_wifi()? {
                continue;
            }
            if self.sound_rx.is_some() && self.poll_sound()? {
                continue;
            }
            if self.sound_test_rx.is_some() && self.poll_sound_test()? {
                continue;
            }
            let event = match self.c.poll_for_event()? {
                Some(event) => event,
                None => {
                    if self.wifi_rx.is_some()
                        || self.sound_rx.is_some()
                        || self.sound_test_rx.is_some()
                    {
                        std::thread::sleep(std::time::Duration::from_millis(40));
                        continue;
                    }
                    self.c.flush()?;
                    self.c.wait_for_event()?
                }
            };
            match event {
                Event::KeyPress(e) => {
                    let (key, shift) = key_text(&self.c, e.detail, e.state);
                    if self.wifi_secret.is_some() {
                        self.wifi_secret_key(key)?;
                        continue;
                    }
                    // While a layout profile is being named, the keys belong
                    // to the name and nothing else.
                    if self.profile_name.is_some() {
                        self.name_key(key)?;
                        continue;
                    }
                    if self.number_edit.is_some() {
                        self.number_key(key)?;
                        continue;
                    }
                    // A notice from the last action lasts until the next key,
                    // so the footer returns to the normal hint by itself.
                    self.error.clear();
                    if e.state.contains(KeyButMask::CONTROL) && key == 0x72 && self.mode == "wifi" {
                        self.start_wifi_job("Scanning for networks…".into(), || Ok(()))?;
                        continue;
                    }
                    if e.state.contains(KeyButMask::CONTROL) && key == 0x70 {
                        self.toggle_favorite()?;
                        continue;
                    }
                    if e.state.contains(KeyButMask::CONTROL) && key == 0x73 {
                        self.save_selected_action()?;
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

/// Only a non-empty spawn row can become a saved action.
fn savable_argv(target: &Target) -> std::result::Result<&[String], &'static str> {
    match target {
        Target::Spawn { argv, .. } if !argv.is_empty() => Ok(argv),
        Target::Spawn { .. } => Err("This row has no command to save"),
        Target::Terminal { .. } => Err("Terminal actions cannot be saved as background actions"),
        Target::Confirm(_) => Err("Confirmation actions are built in and cannot be saved"),
        Target::Mode(_) => Err("View switches cannot be saved as actions"),
        Target::SoundTest(_) => Err("Microphone tests cannot be saved as actions"),
        Target::Info => Err("Info rows cannot be saved as actions"),
        Target::Focus(_) => Err("Window switches cannot be saved as actions"),
    }
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

/// Launch a terminal command. Informational actions keep the terminal open
/// after the child exits, with its exit status, so a report is not lost when
/// kitty/alacritty/foot closes the window immediately.
fn spawn_terminal_command(
    argv: &[String],
    hold: bool,
    cfg: &Config,
) -> std::result::Result<(), String> {
    if argv.is_empty() {
        return Err("empty command".into());
    }
    let mut full = cfg.terminal.clone();
    if hold {
        const HOLD: &str = "status=0; \"$@\" || status=$?; printf '\\n[exit %s] Press Enter to close...\\n' \"$status\"; read -r _ || :; exit \"$status\"";
        full.extend(["sh", "-c", HOLD, "sh"].into_iter().map(str::to_string));
    }
    full.extend(argv.iter().cloned());
    let program = full.remove(0);
    Command::new(&program)
        .args(&full)
        .stdin(Stdio::null())
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
        "--wifi-status" | "--wifi-scan" => {
            let snapshot = wifi::snapshot(arg == "--wifi-scan");
            serde_json::to_string_pretty(&snapshot)
                .map(|text| println!("{text}"))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                .map(|_| ())
        }
        "--apps" | "--actions" | "--system-actions" | "--windows" | "--microphone"
        | "--bluetooth" | "--displays" | "--power" | "--keys" | "--settings" | "--themes"
        | "--timing" | "--input" | "--wifi" | "--sound" => {
            let mode = match arg {
                "--actions" => "actions",
                "--system-actions" => "system-actions",
                "--windows" => "windows",
                "--microphone" => "microphone",
                "--bluetooth" => "bluetooth",
                "--displays" => "displays",
                "--power" => "power",
                "--keys" => "keys",
                "--settings" => "settings",
                "--themes" => "themes",
                "--timing" => "timing",
                "--input" => "input",
                "--wifi" => "wifi",
                "--sound" => "sound",
                _ => "apps",
            };
            match Ui::new(mode) {
                Ok(mut ui) => ui.run(),
                Err(e) => Err(e),
            }
        }
        "--help" => {
            println!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--system-actions|--windows|--microphone|--bluetooth|--displays|--power|--keys|--settings|--themes|--timing|--input|--wifi|--sound|--wifi-status|--wifi-scan|--check-config|--list|--list-actions|--list-search|--version]"
            );
            Ok(())
        }
        _ => {
            eprintln!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--system-actions|--windows|--microphone|--bluetooth|--displays|--power|--sound|--check-config|--list|--list-actions|--list-search|--version]"
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
            keywords: String::new(),
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
    fn suspend_and_search_shortcuts_have_readable_names() {
        assert_eq!(super::shortcut_action_label("suspend"), "Lock and suspend");
        assert_eq!(
            super::shortcut_action_label("launcher_search"),
            "Web search"
        );
    }

    #[test]
    fn the_settings_group_holds_every_settings_view() {
        let items = super::settings_items();
        for key in [
            "settings:themes",
            "settings:displays",
            "settings:audio",
            "settings:keys",
            "settings:timing",
        ] {
            let item = items
                .iter()
                .find(|item| item.key == key)
                .unwrap_or_else(|| panic!("missing {key}"));
            assert_eq!(item.group, "NIRUX Settings");
            if matches!(key, "settings:themes" | "settings:displays") {
                assert!(matches!(item.target_kind(), "mode" | "spawn"));
            } else {
                assert_eq!(item.target_kind(), "mode");
            }
        }
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

    fn items_with_wallpaper_and_nirux() -> Vec<super::Item> {
        let dir = std::env::temp_dir().join(format!("nl-wp-nirux-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("niruwallpaper"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("nirux"), "#!/bin/sh\n").unwrap();
        let previous = std::env::var_os("PATH");
        let home = std::env::var_os("HOME");
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::env::set_var("PATH", &dir);
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
    fn the_focused_action_view_puts_maintenance_before_appearance() {
        let items = vec![
            super::Item {
                key: "theme:x".into(),
                group: "Appearance".into(),
                name: "Theme".into(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: super::Target::Info,
            },
            super::Item {
                key: "maintenance:x".into(),
                group: "NIRU Actions".into(),
                name: "Review storage".into(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: super::Target::Info,
            },
        ];
        let usage = std::collections::HashMap::new();
        let order = super::filtered_for_mode(&items, "", &usage, "system-actions");
        assert_eq!(order.first(), Some(&1));
    }

    #[test]
    fn system_actions_cover_maintenance_without_destructive_defaults() {
        let _guard = lock();
        let items = items_with_wallpaper(true);
        let keys: Vec<&str> = items
            .iter()
            .filter(|item| item.group == "NIRU Actions")
            .map(|item| item.key.as_str())
            .collect();
        assert!(keys.contains(&"action:maintenance:storage-review"));
        assert!(keys.contains(&"action:maintenance:windows-rescue"));
        assert!(keys.contains(&"action:maintenance:tasks"));
        assert!(keys.contains(&"action:agent:unavailable"));
        assert!(
            !keys.contains(&"action:maintenance:cache-clean"),
            "cache cleanup must not run without confirmation"
        );
    }

    #[test]
    fn the_extra_wallpaper_download_is_a_separate_row_when_nirux_exists() {
        let _guard = lock();
        let items = items_with_wallpaper_and_nirux();
        let rows: Vec<_> = items.iter().filter(|i| i.group == "Wallpaper").collect();
        let keys: Vec<&str> = rows.iter().map(|i| i.key.as_str()).collect();
        assert!(keys.contains(&"action:wallpaper:open"));
        assert!(keys.contains(&"action:wallpaper:download-extras"));
        let extra = rows
            .iter()
            .find(|i| i.key == "action:wallpaper:download-extras")
            .unwrap();
        assert!(extra.name.contains("additional"));
        assert_eq!(extra.target_kind(), "terminal");
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
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }
    }

    /// A row is found by its group name, because the group is part of what the
    /// list calls it. Several groups rely on this instead of repeating the
    /// word in every row's name.
    #[test]
    fn an_empty_query_stays_alphabetical_not_shortest_first() {
        let items = vec![
            row("Applications", "Zeta", "z.desktop"),
            row("Applications", "Alpha", "a.desktop"),
        ];
        let usage = std::collections::HashMap::new();
        let order = filtered(&items, "", &usage);
        let names: Vec<&str> = order.iter().map(|&i| items[i].name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Zeta"]);
    }

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

    /// The dedicated mode lists every provider before anything is typed, and
    /// no applications at all.
    #[test]
    fn wifi_view_offers_power_and_refresh_without_an_adapter() {
        let rows = super::wifi_items(&super::wifi::Snapshot::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "wifi:absent");
    }

    #[test]
    fn wifi_search_matches_ssids_instead_of_action_labels_and_clears() {
        let snapshot = super::wifi::Snapshot {
            present: true,
            powered: true,
            networks: vec![
                super::wifi::Network {
                    ssid: "Räknät hemma".into(),
                    security: "WPA2".into(),
                    ..Default::default()
                },
                super::wifi::Network {
                    ssid: "Other".into(),
                    security: "WPA2".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rows = super::wifi_items(&snapshot);
        let matches = super::wifi_filtered(&rows, "RÄKNÄT");
        assert_eq!(matches.len(), 1);
        assert_eq!(rows[matches[0]].name, "Räknät hemma");
        assert!(super::wifi_filtered(&rows, "WPA2").is_empty());
        assert_eq!(super::wifi_filtered(&rows, "").len(), rows.len());
    }

    #[test]
    fn wifi_view_marks_saved_networks_and_skips_the_active_one() {
        let snapshot = super::wifi::Snapshot {
            present: true,
            interface: "wlan0".into(),
            powered: true,
            connected: true,
            ssid: "Home".into(),
            signal: Some(70),
            security: "WPA2".into(),
            saved: vec!["Home".into(), "Guest".into()],
            networks: vec![
                super::wifi::Network {
                    ssid: "Home".into(),
                    active: true,
                    saved: true,
                    ..Default::default()
                },
                super::wifi::Network {
                    ssid: "Guest".into(),
                    saved: true,
                    security: "WPA2".into(),
                    signal: Some(45),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rows = super::wifi_items(&snapshot);
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert!(keys.contains(&"wifi:power"));
        assert!(keys.contains(&"wifi:disconnect"));
        assert!(keys.contains(&"wifi:forget:Home"));
        assert!(keys.contains(&"wifi:connect:Guest"));
        assert!(keys.contains(&"wifi:connect-pw:Guest"));
        assert!(!keys.contains(&"wifi:connect:Home"));
        assert!(keys.contains(&"wifi:refresh"));
    }

    #[test]
    fn an_empty_search_lists_only_providers() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        let rows = super::web_rows("", &catalog);
        assert_eq!(rows.len(), catalog.sources.len());
        assert!(rows
            .iter()
            .all(|item| item.group == "Search" && item.target_kind() == "info"));
        assert!(!rows.iter().any(|item| item.group == "Applications"));
    }

    /// One character is a search; Swedish letters need no extra prefix.
    #[test]
    fn one_character_and_swedish_letters_are_web_searches() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        for text in ["C", "R", "X", "å", "ö"] {
            let rows = super::web_rows(text, &catalog);
            assert_eq!(rows.len(), catalog.sources.len(), "{text}");
            assert!(
                rows.iter().all(|item| item.target_kind() == "spawn"),
                "{text} must open an engine"
            );
            assert!(
                rows.iter().all(|item| item.name.ends_with(text)),
                "{text} must be in every row"
            );
        }
    }

    #[test]
    fn an_address_is_the_first_row() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        let rows = super::web_rows("https://example.com/a?q=1", &catalog);
        assert_eq!(rows[0].key, "search:address");
        assert_eq!(rows[0].name, "Open address");
        assert!(rows[1..].iter().all(|item| item.group == "Search"));
    }

    #[test]
    fn an_alias_selects_one_provider_without_the_prefix() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        let rows = super::web_rows("gh niruwm", &catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "GitHub: niruwm");
        assert!(!rows[0].name.contains("gh "));
    }

    #[test]
    fn an_empty_prefixed_query_does_not_execute() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        let rows = super::web_rows("gh ", &catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_kind(), "info");
        assert_eq!(rows[0].name, "GitHub");
    }

    #[test]
    fn a_provider_name_is_not_stolen_as_an_alias() {
        let catalog = super::websearch::catalog(&[], &[], None, &[]);
        let rows = super::web_rows("github niruwm", &catalog);
        assert_eq!(
            rows.len(),
            catalog.sources.len(),
            "a full name is an ordinary word"
        );
        assert!(rows.iter().all(|item| item.name.ends_with("github niruwm")));
    }

    #[test]
    fn multiword_keyword_and_fuzzy_matching() {
        let mut firefox = row("Applications", "Firefox", "firefox.desktop");
        firefox.keywords = "web browser internet".into();
        let items = vec![firefox, row("Applications", "Files", "files.desktop")];
        let usage = std::collections::HashMap::new();
        assert_eq!(filtered(&items, "web browser", &usage), vec![0]);
        assert_eq!(filtered(&items, "firfox", &usage), vec![0]);
        assert!(filtered(&items, "web files", &usage).is_empty());
    }
}

#[cfg(test)]
mod intent_launcher_tests {
    use super::{
        intent, intent_first_order, intent_item, intent_items, savable_argv, Action, Target,
    };

    #[test]
    fn intent_rows_rank_before_ordinary_results_and_empty_keeps_order() {
        assert_eq!(intent_first_order(2, 2, vec![1, 0]), vec![2, 3, 1, 0]);
        assert_eq!(intent_first_order(2, 0, vec![1, 0]), vec![1, 0]);
    }

    fn spawn(argv: &[&str]) -> Target {
        Target::Spawn {
            argv: argv.iter().map(|arg| (*arg).to_string()).collect(),
            terminal: false,
            path: None,
        }
    }

    #[test]
    fn strong_phrases_resolve_to_fixed_commands() {
        let rows = intent_items("volume up");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group, "Intent");
        assert_eq!(rows[0].name, "Volume up");
        match &rows[0].target {
            Target::Spawn { argv, .. } => {
                assert_eq!(argv, &["niructl", "audio", "volume", "up"]);
                assert!(!argv.iter().any(|arg| arg.contains("volume up")));
            }
            _ => panic!("volume up must be a spawn"),
        }

        let rows = intent_items("starta om");
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].target, Target::Confirm(Action::Reboot)));
        assert!(rows[0].detail.contains("confirmation"));
    }

    #[test]
    fn unavailable_wifi_is_info_and_available_wifi_uses_networkmanager() {
        let on = intent::IntentMatch {
            action: intent::IntentAction::WifiOn,
            phrase: "wifi on",
        };
        let unavailable = intent_item(&on, false);
        assert!(matches!(unavailable.target, Target::Info));
        assert!(unavailable.detail.contains("Unavailable"));

        let available = intent_item(&on, true);
        match &available.target {
            Target::Spawn { argv, .. } => {
                assert_eq!(argv, &["nmcli", "radio", "wifi", "on"]);
            }
            _ => panic!("available Wi-Fi must use nmcli"),
        }
    }

    #[test]
    fn hostile_text_does_not_become_an_intent_or_argv() {
        for query in [
            "lock; touch /tmp/pwned",
            "$(reboot)",
            "volume up && id",
            &"lås ".repeat(200),
        ] {
            assert!(intent_items(query).is_empty(), "{query:?}");
        }
    }

    #[test]
    fn only_non_empty_spawn_rows_can_be_saved() {
        assert_eq!(
            savable_argv(&spawn(&["echo", "ok"])).unwrap(),
            ["echo", "ok"]
        );
        assert!(savable_argv(&spawn(&[])).is_err());
        assert!(savable_argv(&Target::Confirm(Action::Logout)).is_err());
        assert!(savable_argv(&Target::Mode("settings".into())).is_err());
        assert!(savable_argv(&Target::Info).is_err());
        assert!(savable_argv(&Target::Focus(0)).is_err());
    }
}
