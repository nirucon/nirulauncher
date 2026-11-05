//! The actions catalogue and the small views built from niructl actions.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn tailscale_items() -> Vec<Item> {
    let peers = match tailscale::peers() {
        Ok(peers) => peers,
        Err(error) => {
            return vec![Item {
                key: "tailscale:unavailable".into(),
                group: "Tailscale".into(),
                name: "Tailscale unavailable".into(),
                detail: error,
                keywords: "tailscale vpn ssh".into(),
                favorite: false,
                target: Target::Info,
            }]
        }
    };
    if peers.is_empty() {
        return vec![Item {
            key: "tailscale:empty".into(),
            group: "Tailscale".into(),
            name: "No online Tailscale machines".into(),
            detail: "Bring a peer online, then reopen this view".into(),
            keywords: "tailscale vpn ssh".into(),
            favorite: false,
            target: Target::Info,
        }];
    }
    peers
        .into_iter()
        .map(|peer| Item {
            key: format!("tailscale:ssh:{}", peer.host),
            group: "Tailscale".into(),
            name: peer.host.clone(),
            detail: if peer.address.is_empty() {
                "online · ssh".into()
            } else {
                format!("{} · ssh", peer.address)
            },
            keywords: format!("tailscale vpn ssh {}", peer.address),
            favorite: false,
            target: Target::Terminal {
                argv: vec!["ssh".into(), peer.host],
                hold: false,
            },
        })
        .collect()
}

pub(crate) fn clipboard_items() -> Vec<Item> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/state")
        });
    let path = base.join("niru/clipboard.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(entries) = entries.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_i64()?;
            let body = entry.get("text")?.as_str()?;
            let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview = if line.chars().count() > 96 {
                format!("{}…", line.chars().take(96).collect::<String>())
            } else {
                line
            };
            Some(Item {
                key: format!("clipboard:{id}"),
                group: "Clipboard".into(),
                name: preview,
                detail: "Copy this entry back to the clipboard".into(),
                keywords: "clipboard history paste".into(),
                favorite: false,
                target: Target::Spawn {
                    argv: vec!["niruclipboard".into(), "copy".into(), id.to_string()],
                    terminal: false,
                    path: None,
                },
            })
        })
        .collect()
}

/// The single row behind the compact OpenCode prompt view. The query itself
/// is the prompt; the row is a stable focus target and an explanation.
pub(crate) fn agent_items() -> Vec<Item> {
    vec![Item {
        key: "agent:prompt".into(),
        group: "OpenCode".into(),
        name: "Start terminal".into(),
        detail: "Type above · Enter opens the TUI".into(),
        keywords: "agent opencode ai ask prompt".into(),
        favorite: false,
        target: Target::Info,
    }]
}

pub(crate) fn share_items() -> Vec<Item> {
    let mut items = Vec::new();
    let pending_file = std::env::var("NIRUSHARE_SEND_FILE")
        .ok()
        .filter(|p| !p.is_empty());
    let payload_args = pending_file
        .as_ref()
        .map(|p| vec!["--file".to_string(), p.clone()])
        .unwrap_or_else(|| vec!["--clipboard".to_string()]);
    let noun = if pending_file.is_some() {
        "image"
    } else {
        "clipboard"
    };
    items.push(Item {
        key: "share:pair:open".into(),
        group: "NIRU Share".into(),
        name: "Pair a device…".into(),
        detail: "Enter a trusted host and pairing token".into(),
        keywords: "share pair trust device".into(),
        favorite: false,
        target: Target::Mode("share-pair".into()),
    });
    let devices = providers::output("nirushare", &["devices"])
        .ok()
        .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    items.push(Item {
        key: "share:clipboard:all".into(),
        group: "NIRU Share".into(),
        name: format!("Share {noun} with all devices"),
        detail: if devices.is_empty() {
            "No trusted devices".to_string()
        } else {
            "Queue to every trusted device".into()
        },
        keywords: "share clipboard image all devices tailscale".into(),
        favorite: false,
        target: Target::Spawn {
            argv: std::iter::once("nirushare".to_string())
                .chain(std::iter::once("send".to_string()))
                .chain(payload_args.clone())
                .chain(std::iter::once("--all".to_string()))
                .collect(),
            terminal: false,
            path: None,
        },
    });
    for device in devices {
        let Some(host) = device.get("host").and_then(|v| v.as_str()) else {
            continue;
        };
        let online = device
            .get("online")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        items.push(Item {
            key: format!("share:revoke:{host}"),
            group: "NIRU Share".into(),
            name: format!("Revoke trust for {host}"),
            detail: "Stop sending and receiving with this device".into(),
            keywords: format!("share revoke untrust remove {host}"),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["nirushare".into(), "revoke".into(), host.to_string()],
                terminal: false,
                path: None,
            },
        });
        items.push(Item {
            key: format!("share:clipboard:{host}"),
            group: "NIRU Share".into(),
            name: format!("Share {noun} with {host}"),
            detail: if online {
                "Online"
            } else {
                "Offline · queued delivery"
            }
            .into(),
            keywords: format!("share clipboard {host} tailscale"),
            favorite: false,
            target: Target::Spawn {
                argv: std::iter::once("nirushare".to_string())
                    .chain(std::iter::once("send".to_string()))
                    .chain(payload_args.clone())
                    .chain(std::iter::once("--to".to_string()))
                    .chain(std::iter::once(host.to_string()))
                    .collect(),
                terminal: false,
                path: None,
            },
        });
    }
    items.push(Item {
        key: "share:clipboard:email".into(),
        group: "NIRU Share".into(),
        name: "Share clipboard by email".into(),
        detail: "Open a draft in the default mail client".into(),
        keywords: "share clipboard email mail draft".into(),
        favorite: false,
        target: Target::Spawn {
            argv: vec!["nirushare".into(), "email".into()],
            terminal: false,
            path: None,
        },
    });
    let inbox = providers::output("nirushare", &["inbox"])
        .ok()
        .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    for item in inbox.iter().take(20) {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("item");
        let sender = item
            .get("sender")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(kind);
        let command = if kind == "text" { "copy" } else { "open" };
        items.push(Item {
            key: format!("share:inbox:{id}"),
            group: "NIRU Share".into(),
            name: format!(
                "{} {} from {}",
                if command == "copy" { "Copy" } else { "Open" },
                name,
                sender
            ),
            detail: format!("Received {kind}"),
            keywords: format!("share inbox received {sender} {kind}"),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["nirushare".into(), command.into(), id.into()],
                terminal: false,
                path: None,
            },
        });
    }
    items
}

pub(crate) fn share_pair_items() -> Vec<Item> {
    vec![Item {
        key: "share:pair".into(),
        group: "NIRU Share".into(),
        name: "Pair a trusted device".into(),
        detail: "Type: host token · obtain token from the other device".into(),
        keywords: "share pair trust device token tailscale".into(),
        favorite: false,
        target: Target::Info,
    }]
}

pub(crate) fn quick_items() -> Vec<Item> {
    let item = |key: &str, name: &str, detail: &str, target: Target| Item {
        key: key.into(),
        group: "Quick Settings".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: "quick settings control toggle".into(),
        favorite: false,
        target,
    };
    vec![
        item(
            "quick:volume-up",
            "Volume up",
            "Raise output volume",
            spawn_niructl(&["audio", "volume", "up"]),
        ),
        item(
            "quick:volume-down",
            "Volume down",
            "Lower output volume",
            spawn_niructl(&["audio", "volume", "down"]),
        ),
        item(
            "quick:volume-mute",
            "Toggle output mute",
            "Mute or unmute the output",
            spawn_niructl(&["audio", "mute"]),
        ),
        item(
            "quick:mic-mute",
            "Toggle microphone mute",
            "Mute or unmute the input",
            spawn_niructl(&["audio", "mic-mute", "toggle"]),
        ),
        item(
            "quick:wifi",
            "Wi-Fi and network",
            "Open the Wi-Fi selector",
            Target::Mode("wifi".into()),
        ),
        if wifi::wired_present() {
            item(
                "quick:wired",
                "Wired network",
                "Connect or disconnect dock/ethernet without touching Wi-Fi",
                Target::Mode("wired".into()),
            )
        } else {
            item(
                "quick:wired:absent",
                "Wired network",
                "No Ethernet interface is present",
                Target::Info,
            )
        },
        item(
            "quick:bluetooth",
            "Bluetooth",
            "Open Bluetooth devices",
            Target::Mode("bluetooth".into()),
        ),
        item(
            "quick:displays",
            "Displays",
            "Open display profiles and rotation",
            Target::Mode("displays".into()),
        ),
        item(
            "quick:power",
            "Power profile",
            "Choose Power saver, Balanced or Performance",
            Target::Mode("power".into()),
        ),
        item(
            "quick:themes",
            "Themes",
            "Choose a NIRUX theme",
            Target::Mode("themes".into()),
        ),
        item(
            "quick:bar-top",
            "Bar: top",
            "Place the NIRUBAR panel at the top",
            spawn_niructl(&["bar", "top"]),
        ),
        item(
            "quick:bar-bottom",
            "Bar: bottom",
            "Place the NIRUBAR panel at the bottom",
            spawn_niructl(&["bar", "bottom"]),
        ),
        item(
            "quick:bar-hide",
            "Bar: hidden",
            "Hide the bar until you show a bar edge again",
            spawn_niructl(&["bar", "hide"]),
        ),
        item(
            "quick:bar-show",
            "Bar: show",
            "Show the bar at its configured edge",
            spawn_niructl(&["bar", "show"]),
        ),
        item(
            "quick:bar-title",
            "Bar: window title",
            "Show or hide the active window title",
            Target::Spawn {
                argv: vec!["nirubar".into(), "--toggle-title".into()],
                terminal: false,
                path: None,
            },
        ),
        item(
            "quick:sound",
            "Audio devices",
            "Choose output and input devices",
            Target::Mode("sound".into()),
        ),
        item(
            "quick:dnd",
            "Toggle Do Not Disturb",
            "Suppress notification popups",
            spawn_niructl(&["dnd", "toggle"]),
        ),
        item(
            "quick:nightlight",
            "Toggle night light",
            "Warm the display for evening",
            spawn_niructl(&["nightlight", "toggle"]),
        ),
        item(
            "quick:idle",
            "Toggle idle inhibition",
            "Keep the screen awake while active",
            spawn_niructl(&["idle", "toggle"]),
        ),
        item(
            "quick:lock",
            "Lock screen",
            "Lock the current session",
            spawn_niructl(&["lock"]),
        ),
        item(
            "quick:update-restart",
            "Update and restart",
            "Update system, NIRUX and extras, then restart",
            Target::Confirm(Action::UpdateReboot),
        ),
        item(
            "quick:update-shutdown",
            "Update and shut down",
            "Update system, NIRUX and extras, then power off",
            Target::Confirm(Action::UpdateShutdown),
        ),
    ]
}

pub(crate) fn power_items() -> Vec<Item> {
    let Ok(output) = providers::output("niructl", &["powerprofile", "status"]) else {
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

pub(crate) fn spawn_niructl(args: &[&str]) -> Target {
    Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|arg| (*arg).to_string()))
            .collect(),
        terminal: false,
        path: None,
    }
}

pub(crate) fn saved_action_item(action: saved::SavedAction) -> Item {
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
pub(crate) fn append_saved_actions(items: &mut Vec<Item>, actions: &[saved::SavedAction]) {
    let mut keys: std::collections::HashSet<String> =
        items.iter().map(|item| item.key.clone()).collect();
    for action in actions {
        let item = saved_action_item(action.clone());
        if keys.insert(item.key.clone()) {
            items.push(item);
        }
    }
}

pub(crate) fn intent_terminal(args: &[&str]) -> Target {
    Target::Terminal {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|arg| (*arg).to_string()))
            .collect(),
        hold: true,
    }
}

pub(crate) fn intent_item(found: &intent::IntentMatch, nmcli: bool) -> Item {
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

pub(crate) fn intent_items(query: &str) -> Vec<Item> {
    let nmcli = in_path("nmcli");
    intent::resolve(query)
        .into_iter()
        .map(|found| intent_item(&found, nmcli))
        .collect()
}

pub(crate) fn sound_action_items() -> Vec<Item> {
    let mut items = Vec::new();
    for (key, name, detail, target) in [
        (
            "sound:open",
            "Open Sound…",
            "Output, input, applications, event sounds and microphone test",
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

/// Every row of the actions view, in its historical order. The view itself
/// shows [`action_static_items`] at once and loads [`action_provider_items`]
/// in a worker; `--list-actions` and the tests use this complete list.
pub(crate) fn action_items(cfg: &Config) -> Vec<Item> {
    let mut items = action_command_rows();
    items.extend(clip_items());
    items.extend(sound_action_items());
    items.extend(theme_items());
    items.extend(audio_output_rows());
    items.extend(shortcut_items());
    items.extend(settings_items());
    items.extend(input_items());
    items.extend(microphone_items());
    items.extend(bluetooth_items());
    items.extend(display_items());
    items.extend(power_items());
    items.extend(diagnostic_rows());
    items.extend(update_rows());
    items.extend(recipe_rows());
    items.extend(action_tail_rows(cfg));
    items
}

/// The action rows that need no provider query: shown in the first paint.
pub(crate) fn action_static_items(cfg: &Config) -> Vec<Item> {
    let mut items = action_command_rows();
    items.extend(clip_static_items());
    items.extend(sound_action_items());
    items.extend(settings_items());
    items.extend(update_rows());
    items.extend(action_tail_rows(cfg));
    items
}

/// The action rows that come from providers (niructl, niruclip, wpctl).
pub(crate) fn action_provider_items() -> Vec<Item> {
    let mut items = clip_provider_items();
    items.extend(theme_items());
    items.extend(audio_output_rows());
    items.extend(shortcut_items());
    items.extend(input_items());
    items.extend(microphone_items());
    items.extend(bluetooth_items());
    items.extend(display_items());
    items.extend(power_items());
    items.extend(diagnostic_rows());
    items.extend(recipe_rows());
    items
}

pub(crate) fn action_command_rows() -> Vec<Item> {
    let mut items = Vec::new();
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
    items
}

pub(crate) fn audio_output_rows() -> Vec<Item> {
    let mut items = Vec::new();
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };
    // Choosing an image, the scale, rotation and theme following belong to
    // NIRU Wallpaper, which shows previews and the per-screen choice. The
    // launcher keeps the one entry that opens it instead of repeating a list
    // of files here; themes stay in this launcher, under Appearance.

    // Audio output/source selection uses niructl -> wpctl. The ID, not the
    // display name, is authoritative because two devices may share a name.
    for (kind, label) in [("outputs", "Audio output")] {
        if let Ok(out) = providers::output("niructl", &["audio", kind]) {
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
    items
}

pub(crate) fn diagnostic_rows() -> Vec<Item> {
    let mut items = Vec::new();
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };

    if let Ok(out) = providers::output("niructl", &["explain", "--launcher"]) {
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
    items
}

pub(crate) fn update_rows() -> Vec<Item> {
    let mut items = Vec::new();
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
    items
}

pub(crate) fn recipe_rows() -> Vec<Item> {
    let mut items = Vec::new();
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };

    if let Ok(out) = providers::output("niructl", &["recipe", "list"]) {
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
    items
}

pub(crate) fn action_tail_rows(cfg: &Config) -> Vec<Item> {
    let mut items = Vec::new();
    let niructl = |args: &[&str]| Target::Spawn {
        argv: std::iter::once("niructl".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect(),
        terminal: false,
        path: None,
    };

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
        (
            "Update and restart…",
            "Updates system, NIRUX and extras in a terminal, then restarts",
            Action::UpdateReboot,
        ),
        (
            "Update and shut down…",
            "Updates system, NIRUX and extras in a terminal, then powers off",
            Action::UpdateShutdown,
        ),
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
    if in_path("nirupres") {
        items.push(Item {
            key: "action:nirupres".into(),
            group: "Presentation".into(),
            name: "Open NIRUPRES".into(),
            detail: "Presentation studio · Follow NIRUX chrome and Talk recording".into(),
            keywords: "presentation slides deck pdf talk record nirupres".into(),
            favorite: false,
            target: Target::Spawn {
                argv: vec!["nirupres".into()],
                terminal: false,
                path: None,
            },
        });
    }
    items.push(Item {
        key: "action:health-view".into(),
        group: "System".into(),
        name: "Health overview".into(),
        detail: "Installed/running state, audio, network, display and capture".into(),
        keywords: "health status doctor diagnostics".into(),
        favorite: false,
        target: Target::Mode("health".into()),
    });
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

/// The `=` row for an arithmetic query, if the query is one.
pub(crate) fn calc_row(query: &str) -> Option<Item> {
    let value = calc::format(calc::evaluate(query)?);
    Some(Item {
        key: "calc:result".into(),
        group: String::new(),
        name: format!("= {value}"),
        detail: "Enter copies the result".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Copy(value),
    })
}

#[cfg(test)]
mod wallpaper_tests {
    use crate::{action_items, filtered, Config};

    fn items_with_wallpaper(installed: bool) -> Vec<crate::Item> {
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

    fn items_with_wallpaper_and_nirux() -> Vec<crate::Item> {
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
            crate::Item {
                key: "theme:x".into(),
                group: "Appearance".into(),
                name: "Theme".into(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: crate::Target::Info,
            },
            crate::Item {
                key: "maintenance:x".into(),
                group: "NIRU Actions".into(),
                name: "Review storage".into(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: crate::Target::Info,
            },
        ];
        let usage = crate::usage::Usage::default();
        let order = crate::filtered_for_mode(&items, "", &usage, "system-actions");
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
        let usage = crate::usage::Usage::default();
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
mod intent_launcher_tests {
    use crate::wifi;
    use crate::{
        agent_command, filtered_for_mode, intent, intent_first_order, intent_item, intent_items,
        savable_argv, Action, Config, Target,
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
    fn an_agent_prompt_is_one_argument_and_keeps_the_class() {
        let cfg = Config::default();
        let prompt = "sammanfatta \"senaste\" ; echo $HOME";
        let argv = agent_command(prompt, &cfg);
        assert_eq!(
            &argv[..5],
            ["kitty", "--class", "niru-agent", "niructl", "agent"]
        );
        assert_eq!(argv.last().map(String::as_str), Some(prompt));
        assert_eq!(argv.len(), 6, "the prompt must never be split");
    }

    #[test]
    fn the_agent_view_keeps_its_row_while_the_prompt_is_typed() {
        let rows = crate::agent_items();
        let usage = crate::usage::Usage::default();
        assert_eq!(
            filtered_for_mode(&rows, "skriv en kort prompt", &usage, "agent"),
            vec![0]
        );
    }

    #[test]
    fn quick_settings_has_update_and_power_actions() {
        let rows = crate::quick_items();
        let shutdown = rows
            .iter()
            .find(|row| row.key == "quick:update-shutdown")
            .expect("update and shut down row");
        let restart = rows
            .iter()
            .find(|row| row.key == "quick:update-restart")
            .expect("update and restart row");
        assert!(matches!(
            shutdown.target,
            Target::Confirm(Action::UpdateShutdown)
        ));
        assert!(matches!(
            restart.target,
            Target::Confirm(Action::UpdateReboot)
        ));
    }

    #[test]
    fn wired_rows_offer_only_per_interface_actions() {
        let snapshot = wifi::WiredSnapshot {
            available: true,
            manager: "NetworkManager".into(),
            devices: vec![
                wifi::WiredDevice {
                    interface: "enp1s0".into(),
                    state: "connected".into(),
                    connected: true,
                    carrier: true,
                    managed: true,
                    address: "192.0.2.20/24".into(),
                    connection: "Dock".into(),
                    speed: Some(1000),
                },
                wifi::WiredDevice {
                    interface: "enp2s0".into(),
                    state: "unavailable".into(),
                    carrier: false,
                    managed: true,
                    ..Default::default()
                },
            ],
            error: String::new(),
        };
        let rows = crate::modes::network::wired_items(&snapshot);
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        assert!(keys.contains(&"wired:disconnect:enp1s0"));
        assert!(keys.contains(&"wired:connect:enp2s0"));
        assert!(!keys
            .iter()
            .any(|key| key.contains("wifi") || key.contains("radio")));
        assert!(rows
            .iter()
            .find(|row| row.key == "wired:disconnect:enp1s0")
            .unwrap()
            .detail
            .contains("Wi-Fi stays untouched"));
        let read_only = wifi::WiredSnapshot {
            available: true,
            manager: "read-only (no NetworkManager)".into(),
            devices: vec![wifi::WiredDevice {
                interface: "enp1s0".into(),
                carrier: true,
                managed: false,
                ..Default::default()
            }],
            error: String::new(),
        };
        let rows = crate::modes::network::wired_items(&read_only);
        assert!(!rows.iter().any(|row| row.key.starts_with("wired:connect:")));
        assert!(rows.iter().any(|row| row.name.contains("read-only")));
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

#[cfg(test)]
mod input_tests {
    #![allow(dead_code, unused_imports)]
    use crate::{apply_caps, keyboard_level, keyboard_symbol};

    use x11rb::protocol::xproto::KeyButMask;

    #[test]
    fn action_list_includes_workspace_ten() {
        let items = crate::action_items(&crate::config::Config::default());
        assert!(items.iter().any(|item| {
            item.key == "action:workspace:10"
                && item.group == "Workspaces"
                && item.name == "Workspace 10"
        }));
        assert!(items.iter().any(|item| item.key == "action:reset-bar"));
    }
}
