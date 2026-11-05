//! Settings views: displays, Bluetooth, themes, keybindings, timing and input.

use crate::*;

pub(crate) fn bluetooth_items() -> Vec<Item> {
    let mut items = Vec::new();
    let Some(output) = bounded_output(
        "niructl",
        &["bluetooth", "status"],
        std::time::Duration::from_millis(250),
    ) else {
        return vec![bluetooth_info(
            "Bluetooth unavailable",
            "niructl did not respond within 250 ms",
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

pub(crate) fn display_items() -> Vec<Item> {
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
    if let Some(output) = bounded_output(
        "niructl",
        &["display", "list"],
        std::time::Duration::from_millis(250),
    ) {
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
    if let Some(output) = bounded_output(
        "niructl",
        &["monitors"],
        std::time::Duration::from_millis(250),
    ) {
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
    if let Some(output) = bounded_output(
        "niructl",
        &["display", "status", "--json"],
        std::time::Duration::from_millis(250),
    ) {
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

pub(crate) fn bluetooth_info(name: &str, detail: &str) -> Item {
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

pub(crate) fn shortcut_key_label(raw: &str) -> String {
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

pub(crate) fn shortcut_action_label(action: &str) -> String {
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

pub(crate) fn theme_items() -> Vec<Item> {
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
    if let Some(output) = bounded_output(
        "niructl",
        &["theme-catalog"],
        std::time::Duration::from_millis(250),
    ) {
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
pub(crate) fn shortcut_items() -> Vec<Item> {
    let mut shortcuts: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(out) = bounded_output("niructl", &["keys"], std::time::Duration::from_millis(250)) {
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
pub(crate) fn settings_items() -> Vec<Item> {
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
pub(crate) fn timing_items() -> Vec<Item> {
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
    match providers::output("niructl", &["timing", "show"]) {
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

pub(crate) fn input_items() -> Vec<Item> {
    let out = match providers::output("niructl", &["input", "status", "--json"]) {
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

#[cfg(test)]
mod settings_tests {

    #[test]
    fn suspend_and_search_shortcuts_have_readable_names() {
        assert_eq!(crate::shortcut_action_label("suspend"), "Lock and suspend");
        assert_eq!(
            crate::shortcut_action_label("launcher_search"),
            "Web search"
        );
    }

    #[test]
    fn the_settings_group_holds_every_settings_view() {
        let items = crate::settings_items();
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
}
