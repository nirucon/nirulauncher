//! The Health overview.

#[allow(unused_imports)]
use crate::*;

/// The Health rows. The view shows [`health_static_items`] at once and these
/// provider rows when they arrive; each check runs in its own thread so the
/// slowest provider, not their sum, decides when the rows appear.
pub(crate) fn health_provider_items() -> Vec<Item> {
    let info = |key: &str, group: &str, name: String, detail: String| Item {
        key: key.into(),
        group: group.into(),
        name,
        detail,
        keywords: "health status diagnostics installed running".into(),
        favorite: false,
        target: Target::Info,
    };
    let info = &info;
    std::thread::scope(|scope| {
        let sections = [
            scope.spawn(|| {
                let mut items = Vec::new();
                match providers::output("nirux", &["status"]) {
                    Ok(output) if output.status.success() => {
                        for (index, line) in
                            String::from_utf8_lossy(&output.stdout).lines().enumerate()
                        {
                            if line.starts_with("installed ") {
                                continue;
                            }
                            let key = if index == 0 {
                                "health:nirux".to_string()
                            } else {
                                format!("health:nirux:{index}")
                            };
                            let target = if line.contains("old generation") {
                                Target::Terminal {
                                    argv: vec!["nirux".into(), "status".into()],
                                    hold: true,
                                }
                            } else {
                                Target::Info
                            };
                            items.push(Item {
                                key,
                                group: "Health".into(),
                                name: line.to_string(),
                                detail: if line.contains("old generation") {
                                    "Log out and back in to activate".into()
                                } else {
                                    "NIRUX runtime status".into()
                                },
                                keywords: "health status".into(),
                                favorite: false,
                                target,
                            });
                        }
                    }
                    Ok(_) => items.push(info(
                        "health:nirux",
                        "Health",
                        "NIRUX status needs attention".into(),
                        "Run the full diagnostics for details".into(),
                    )),
                    Err(_) => items.push(info(
                        "health:nirux",
                        "Health",
                        "NIRUX is not available".into(),
                        "The installer command was not found".into(),
                    )),
                }
                items
            }),
            scope.spawn(|| {
                let mut items = Vec::new();
                match sound::status() {
                    Ok(snapshot) if snapshot.available => {
                        let output = snapshot
                            .output
                            .as_ref()
                            .map(|reading| format!("{} ({}%)", reading.name, reading.percent))
                            .unwrap_or_else(|| "no default output".into());
                        let input = snapshot
                            .input
                            .as_ref()
                            .map(|reading| format!("{} ({}%)", reading.name, reading.percent))
                            .unwrap_or_else(|| "no default input".into());
                        items.push(Item {
                            key: "health:audio".into(),
                            group: "Health".into(),
                            name: "Audio available".into(),
                            detail: format!("Output: {output} · Input: {input}"),
                            keywords: "audio pipewire microphone".into(),
                            favorite: false,
                            target: Target::Mode("sound".into()),
                        });
                    }
                    Ok(snapshot) => items.push(Item {
                        key: "health:audio".into(),
                        group: "Health".into(),
                        name: "Audio unavailable".into(),
                        detail: snapshot
                            .reason
                            .unwrap_or_else(|| "PipeWire is not responding".into()),
                        keywords: "audio pipewire microphone".into(),
                        favorite: false,
                        target: Target::Mode("sound".into()),
                    }),
                    Err(error) => items.push(info(
                        "health:audio",
                        "Health",
                        "Audio unavailable".into(),
                        error,
                    )),
                }
                items
            }),
            scope.spawn(|| {
                let mut items = Vec::new();
                let network = wifi::snapshot(false);
                let network_name = if !network.present {
                    "Network provider unavailable".to_string()
                } else if network.connected {
                    format!(
                        "Network: {}",
                        if network.ssid.is_empty() {
                            "connected"
                        } else {
                            &network.ssid
                        }
                    )
                } else {
                    "Network: disconnected".to_string()
                };
                items.push(Item {
                    key: "health:network".into(),
                    group: "Health".into(),
                    name: network_name,
                    detail: "Open Wi-Fi and connection details".into(),
                    keywords: "network wifi connection".into(),
                    favorite: false,
                    target: Target::Mode("wifi".into()),
                });
                items
            }),
            scope.spawn(|| {
                let mut items = Vec::new();
                let display_ok = providers::output("niructl", &["monitors"])
                    .map(|output| output.status.success())
                    .unwrap_or(false);
                items.push(Item {
                    key: "health:display".into(),
                    group: "Health".into(),
                    name: if display_ok {
                        "Display service available".into()
                    } else {
                        "Display service unavailable".into()
                    },
                    detail: "Open display layout and profiles".into(),
                    keywords: "display monitor randr".into(),
                    favorite: false,
                    target: Target::Mode("displays".into()),
                });
                items
            }),
            scope.spawn(|| {
                let mut items = Vec::new();
                let capture = providers::output("niruclip", &["--status", "--json"])
                    .ok()
                    .and_then(|output| {
                        serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
                    })
                    .and_then(|value| {
                        value
                            .get("state")
                            .and_then(|state| state.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "unavailable".into());
                items.push(Item {
                    key: "health:capture".into(),
                    group: "Health".into(),
                    name: format!("Capture: {capture}"),
                    detail: "Open capture profiles and recording controls".into(),
                    keywords: "capture recording camera niruclip".into(),
                    favorite: false,
                    target: Target::Mode("clip".into()),
                });
                items
            }),
        ];
        sections
            .into_iter()
            .flat_map(|section| section.join().unwrap_or_default())
            .collect()
    })
}

/// Health rows that need no provider: the diagnostics entry.
pub(crate) fn health_static_items() -> Vec<Item> {
    vec![Item {
        key: "health:doctor".into(),
        group: "Health".into(),
        name: "Run full diagnostics".into(),
        detail: "Packages, session, display and runtime checks".into(),
        keywords: "doctor diagnostics".into(),
        favorite: false,
        target: Target::Terminal {
            argv: vec!["nirux".into(), "doctor".into()],
            hold: true,
        },
    }]
}
