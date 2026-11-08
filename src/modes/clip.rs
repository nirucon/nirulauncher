//! Capture (NIRUCLIP) rows.

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

fn clip_status() -> Option<serde_json::Value> {
    let output = providers::output("niruclip", &["--status", "--json"]).ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
}

/// A camera problem NIRUCLIP reports for the running recording (privacy
/// image, frozen picture, no frames, camera busy), if any.
fn camera_problem(status: &serde_json::Value) -> Option<String> {
    let active = matches!(
        status.get("state").and_then(|s| s.as_str()),
        Some("countdown" | "starting" | "recording" | "pausing" | "paused")
    );
    let camera = status.get("camera")?;
    let state = camera.get("status")?.as_str()?;
    if !active || matches!(state, "ok" | "checking") {
        return None;
    }
    Some(
        camera
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or(state)
            .to_string(),
    )
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
        spawn(
            "clip:check-camera",
            "Check camera",
            "Test the camera picture (privacy key, shutter, busy)",
            &["--check-camera"],
        ),
        spawn(
            "clip:preview",
            "Camera preview",
            "Live window with the overlay shape · q closes",
            &["--preview"],
        ),
        spawn(
            "clip:export-mp4",
            "Last recording → MP4",
            "Lossless copy next to the MKV, plays everywhere",
            &["--export", "mp4"],
        ),
        spawn(
            "clip:export-gif",
            "Last recording → GIF",
            "First 30 s, 12 fps, up to 720 px wide",
            &["--export", "gif"],
        ),
        spawn(
            "clip:share",
            "Share last recording",
            "As MP4 via NIRU Share",
            &["--share"],
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
    let status = clip_status();
    if let Some(problem) = status.as_ref().and_then(camera_problem) {
        items.push(Item {
            key: "clip:camera-warning".into(),
            group: "Capture".into(),
            name: "Camera problem".into(),
            detail: problem,
            keywords: "record camera webcam privacy warning".into(),
            favorite: false,
            target: Target::Info,
        });
    }
    for (name, description, mode, audio, webcam) in clip_profiles() {
        items.push(spawn(
            &format!("clip:profile:{name}"),
            &format!("Profile · {description}"),
            &format!("{mode} · {audio} · camera {webcam}"),
            &["--profile", &name, "--countdown", "3"],
        ));
    }
    let recoverable = status
        .as_ref()
        .and_then(|value| value.get("recoverable").and_then(|r| r.as_bool()))
        .unwrap_or(false);
    if recoverable {
        items.push(spawn(
            "clip:recover",
            "Recover interrupted recording",
            "Validate the partial file and restore it",
            &["--recover"],
        ));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::camera_problem;

    #[test]
    fn camera_problems_are_shown_only_for_an_active_recording() {
        let privacy = serde_json::json!({
            "state": "recording",
            "camera": {"device": "/dev/video0", "status": "privacy", "message": "the camera shows its privacy image"}
        });
        assert_eq!(
            camera_problem(&privacy).as_deref(),
            Some("the camera shows its privacy image")
        );
        let ok = serde_json::json!({"state": "recording", "camera": {"device": "/dev/video0", "status": "ok"}});
        assert!(camera_problem(&ok).is_none());
        let finished = serde_json::json!({"state": "finished", "camera": {"device": "/dev/video0", "status": "privacy"}});
        assert!(camera_problem(&finished).is_none());
        assert!(camera_problem(&serde_json::json!({"state": "recording"})).is_none());
    }
}
