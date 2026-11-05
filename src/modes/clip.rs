//! Capture (NIRUCLIP) rows.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn clip_profiles() -> Vec<(String, String, String, String, String)> {
    let Ok(output) = providers::output("niruclip", &["--list-profiles"]) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() >= 5).then(|| {
                (
                    fields[0].to_string(),
                    fields[1].to_string(),
                    fields[2].to_string(),
                    fields[3].to_string(),
                    fields[4].to_string(),
                )
            })
        })
        .collect()
}

pub(crate) fn clip_recoverable() -> bool {
    let Ok(output) = providers::output("niruclip", &["--status", "--json"]) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|value| {
            value
                .get("recoverable")
                .and_then(|recoverable| recoverable.as_bool())
        })
        .unwrap_or(false)
}

pub(crate) fn clip_items() -> Vec<Item> {
    let mut items = clip_static_items();
    items.extend(clip_provider_items());
    items
}

pub(crate) fn clip_static_items() -> Vec<Item> {
    let spawn = |key: &str, name: &str, detail: &str, args: &[&str]| Item {
        key: key.into(),
        group: "Capture".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: "record recording screen webcam capture video spela in".into(),
        favorite: false,
        target: Target::Spawn {
            argv: std::iter::once("niruclip".to_string())
                .chain(args.iter().map(|arg| (*arg).to_string()))
                .collect(),
            terminal: false,
            path: None,
        },
    };
    let items = vec![
        spawn(
            "clip:full",
            "Full screen",
            "Screen + microphone",
            &["--full", "--audio", "mic", "--countdown", "3"],
        ),
        spawn(
            "clip:region",
            "Region",
            "Select area + microphone",
            &["--region", "--audio", "mic", "--countdown", "3"],
        ),
        spawn(
            "clip:webcam",
            "Webcam only",
            "Camera + microphone",
            &["--webcam-only", "--audio", "mic", "--countdown", "3"],
        ),
        spawn(
            "clip:overlay-tl",
            "Cam TL",
            "Full screen + webcam, top left",
            &[
                "--full",
                "--webcam",
                "corner",
                "--corner",
                "top-left",
                "--audio",
                "mic",
                "--countdown",
                "3",
            ],
        ),
        spawn(
            "clip:overlay-tr",
            "Cam TR",
            "Full screen + webcam, top right",
            &[
                "--full",
                "--webcam",
                "corner",
                "--corner",
                "top-right",
                "--audio",
                "mic",
                "--countdown",
                "3",
            ],
        ),
        spawn(
            "clip:overlay-bl",
            "Cam BL",
            "Full screen + webcam, bottom left",
            &[
                "--full",
                "--webcam",
                "corner",
                "--corner",
                "bottom-left",
                "--audio",
                "mic",
                "--countdown",
                "3",
            ],
        ),
        spawn(
            "clip:overlay-br",
            "Cam BR",
            "Full screen + webcam, bottom right",
            &[
                "--full",
                "--webcam",
                "corner",
                "--corner",
                "bottom-right",
                "--audio",
                "mic",
                "--countdown",
                "3",
            ],
        ),
        spawn(
            "clip:stop",
            "Stop",
            "Finalize the current recording",
            &["--stop"],
        ),
        Item {
            key: "clip:status".into(),
            group: "Capture".into(),
            name: "Recording status".into(),
            detail: "niruclip --status".into(),
            keywords: "record recording status".into(),
            favorite: false,
            target: Target::Terminal {
                argv: vec!["niruclip".into(), "--status".into()],
                hold: true,
            },
        },
    ];
    items
}

/// Capture profiles and recovery, from `niruclip`.
pub(crate) fn clip_provider_items() -> Vec<Item> {
    let spawn = |key: &str, name: &str, detail: &str, args: &[&str]| Item {
        key: key.into(),
        group: "Capture".into(),
        name: name.into(),
        detail: detail.into(),
        keywords: "record recording screen webcam capture video spela in".into(),
        favorite: false,
        target: Target::Spawn {
            argv: std::iter::once("niruclip".to_string())
                .chain(args.iter().map(|arg| (*arg).to_string()))
                .collect(),
            terminal: false,
            path: None,
        },
    };
    let mut items = Vec::new();
    for (name, description, mode, audio, webcam) in clip_profiles() {
        items.push(spawn(
            &format!("clip:profile:{name}"),
            &format!("Profile · {description}"),
            &format!("{mode} · {audio} · camera {webcam}"),
            &["--profile", &name, "--countdown", "3"],
        ));
    }
    if clip_recoverable() {
        items.push(spawn(
            "clip:recover",
            "Recover interrupted recording",
            "Validate the partial file and restore it",
            &["--recover"],
        ));
    }
    items
}
