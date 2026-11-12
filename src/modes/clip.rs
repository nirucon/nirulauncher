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
    clip_static_items_with(&[])
}

/// The fixed rows; `mic` (`--mic-channels`, `--mic`) is added to the rows that
/// record the microphone.
fn clip_static_items_with(mic: &[String]) -> Vec<Item> {
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
                .chain(
                    args.contains(&"mic")
                        .then(|| mic.iter().cloned())
                        .into_iter()
                        .flatten(),
                )
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
    // The microphone list (ffmpeg, pw-dump) runs next to the status query.
    let (mics, status) = std::thread::scope(|scope| {
        let mics = scope.spawn(list_mics);
        let status = clip_status();
        (mics.join().unwrap_or_default(), status)
    });
    let inputs = MicContext::new(mics, &ClipConfig::load(), &load_inputs());
    if let Some(row) = inputs.row() {
        items.push(row);
        // The fixed recording rows again, now with the chosen inputs; they
        // replace the rows with the same keys.
        let mic = inputs.static_args();
        if !mic.is_empty() {
            items.extend(
                clip_static_items_with(&mic)
                    .into_iter()
                    .filter(item_records_mic),
            );
        }
    }
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
        let mut args = vec!["--profile", &name, "--countdown", "3"];
        let mic = inputs.profile_args(&name, &audio);
        args.extend(mic.iter().map(String::as_str));
        let mut detail = format!("{mode} · {audio} · camera {webcam}");
        if let Some(value) = mic.get(1) {
            detail.push_str(&format!(" · {}", choice_short(value)));
        }
        items.push(spawn(
            &format!("clip:profile:{name}"),
            &format!("Profile · {description}"),
            &detail,
            &args,
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

/// The view that picks the inputs of one multichannel source.
pub(crate) const CLIP_INPUTS_MODE: &str = "clip-inputs:";
/// A view switch that stores a choice (`VALUE<TAB>SOURCE`) and goes back.
pub(crate) const CLIP_SET_INPUTS: &str = "clip-set-inputs:";
/// NIRUCLIP accepts inputs 1 to 32.
const MAX_INPUTS: usize = 32;

/// A microphone from `niruclip --list-mics --json`. NIRUCLIP before 0.7.0
/// prints no channel fields, so none of its sources is multichannel.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mic {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) default: bool,
    pub(crate) channels: usize,
    pub(crate) channel_names: Vec<String>,
    pub(crate) multichannel: bool,
}

impl Mic {
    fn label(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }
}

pub(crate) fn parse_mics(json: &[u8]) -> Vec<Mic> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_slice(json) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_string);
            let channels = entry.get("channels").and_then(|v| v.as_u64()).unwrap_or(0);
            let channels = (channels as usize).min(MAX_INPUTS);
            Some(Mic {
                name: text("name")?,
                description: text("description").unwrap_or_default(),
                default: entry.get("default").and_then(|v| v.as_bool()) == Some(true),
                channels,
                channel_names: entry
                    .get("channel_names")
                    .and_then(|v| v.as_array())
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                multichannel: entry.get("multichannel").and_then(|v| v.as_bool()) == Some(true)
                    && channels > 2,
            })
        })
        .collect()
}

fn list_mics() -> Vec<Mic> {
    match providers::bounded(
        "niruclip",
        &["--list-mics", "--json"],
        Duration::from_secs(3),
    ) {
        Some(output) if output.status.success() => parse_mics(&output.stdout),
        _ => Vec::new(),
    }
}

/// The keys of NIRUCLIP's config that pick the microphone.
#[derive(Clone, Default)]
pub(crate) struct ClipConfig {
    mic: Option<String>,
    mic_channels: Option<String>,
    /// Profile name, its `mic`, and whether it sets `mic_channels`.
    profiles: Vec<(String, Option<String>, bool)>,
}

impl ClipConfig {
    fn load() -> Self {
        let path = std::env::var_os("NIRUCLIP_CONFIG")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| config_home().join("niruclip/config.toml"));
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub(crate) fn parse(text: &str) -> Self {
        let Ok(value) = text.parse::<toml::Table>() else {
            return Self::default();
        };
        let text = |table: &toml::Table, key: &str| {
            table.get(key).and_then(|v| v.as_str()).map(str::to_string)
        };
        let profiles = value
            .get("profiles")
            .and_then(|v| v.as_table())
            .map(|profiles| {
                profiles
                    .iter()
                    .filter_map(|(name, profile)| {
                        let profile = profile.as_table()?;
                        Some((
                            name.clone(),
                            text(profile, "mic"),
                            profile.contains_key("mic_channels"),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            mic: text(&value, "mic"),
            mic_channels: text(&value, "mic_channels"),
            profiles,
        }
    }
}

fn config_home() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
}

fn inputs_path() -> std::path::PathBuf {
    usage::path().with_file_name("clip-inputs.tsv")
}

/// The remembered choices, `SOURCE<TAB>VALUE` per line.
pub(crate) fn load_inputs() -> Vec<(String, String)> {
    std::fs::read_to_string(inputs_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (source, value) = line.split_once('\t')?;
            (!source.is_empty()).then(|| (source.to_string(), value.trim().to_string()))
        })
        .collect()
}

pub(crate) fn has_input_choices() -> bool {
    !load_inputs().is_empty()
}

/// Remember `value` for `source`, newest last.
pub(crate) fn save_input(source: &str, value: &str) {
    if source.is_empty() || source.contains(['\n', '\t']) || value.contains(['\n', '\t']) {
        return;
    }
    let mut rows = load_inputs();
    rows.retain(|(name, _)| name != source);
    rows.push((source.to_string(), value.to_string()));
    let skip = rows.len().saturating_sub(MAX_INPUTS);
    let text: String = rows
        .iter()
        .skip(skip)
        .map(|(name, value)| format!("{name}\t{value}\n"))
        .collect();
    let path = inputs_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension(format!("tsv.{}", std::process::id()));
    if std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, &path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// A choice NIRUCLIP accepts for a source with `channels` inputs: `all`,
/// `off` (do not record this source), `N` or `N,M`.
fn valid_choice(value: &str, channels: usize) -> bool {
    let input = |text: &str| {
        text.parse::<usize>()
            .is_ok_and(|n| (1..=channels.min(MAX_INPUTS)).contains(&n))
    };
    match value.split_once(',') {
        _ if value == "all" || value == "off" => true,
        Some((left, right)) => input(left) && input(right) && left != right,
        None => input(value),
    }
}

pub(crate) fn choice_label(value: &str, channels: usize) -> String {
    match value.split_once(',') {
        _ if value == "all" && channels > 0 => format!("All {channels} inputs"),
        _ if value == "all" => "All inputs".into(),
        _ if value == "off" => "Not used".into(),
        Some((left, right)) => format!("Inputs {left}+{right} (stereo)"),
        None => format!("Input {value} (mono)"),
    }
}

/// The choice on the Inputs row: `3+4 stereo`, `3 mono`, `all 18`.
fn row_label(value: &str, channels: usize) -> String {
    match value.split_once(',') {
        _ if value == "all" => format!("all {channels}"),
        _ if value == "off" => "not used".into(),
        Some((left, right)) => format!("{left}+{right} stereo"),
        None => format!("{value} mono"),
    }
}

/// The choice in a profile row: `inputs 1+2`, `input 3` or `all inputs`.
fn choice_short(value: &str) -> String {
    match value.split_once(',') {
        _ if value == "all" => "all inputs".into(),
        Some((left, right)) => format!("inputs {left}+{right}"),
        None => format!("input {value}"),
    }
}

/// The microphones, which one a recording uses, and the remembered choices.
pub(crate) struct MicContext {
    mics: Vec<Mic>,
    config: ClipConfig,
    choices: Vec<(String, String)>,
    /// The source a plain recording uses: `mic =` or the system default.
    used: Option<usize>,
    /// The multichannel source the Inputs row is for: the used one, or else
    /// the first multichannel source.
    target: Option<usize>,
}

impl MicContext {
    pub(crate) fn new(mics: Vec<Mic>, config: &ClipConfig, choices: &[(String, String)]) -> Self {
        let mut context = Self {
            mics,
            config: config.clone(),
            choices: choices.to_vec(),
            used: None,
            target: None,
        };
        context.used = context.resolve(context.config.mic.as_deref());
        context.target = context
            .used
            .filter(|&index| context.mics[index].multichannel)
            .or_else(|| context.mics.iter().position(|mic| mic.multichannel));
        context
    }

    /// The source NIRUCLIP records for a `mic =` value, with its matching
    /// rules: the exact name, a whole name or description, then a part of
    /// the description or name; anything else is the system default.
    fn resolve(&self, wanted: Option<&str>) -> Option<usize> {
        let default = || self.mics.iter().position(|mic| mic.default);
        let wanted = wanted.map(str::trim).unwrap_or_default();
        if wanted.is_empty() || wanted == "default" {
            return default();
        }
        let lower = wanted.to_lowercase();
        let find = |test: &dyn Fn(&Mic) -> bool| self.mics.iter().position(test);
        find(&|mic| mic.name == wanted)
            .or_else(|| {
                find(&|mic| {
                    mic.name.to_lowercase() == lower || mic.description.to_lowercase() == lower
                })
            })
            .or_else(|| find(&|mic| mic.description.to_lowercase().contains(&lower)))
            .or_else(|| find(&|mic| mic.name.to_lowercase().contains(&lower)))
            .or_else(default)
    }

    /// The remembered choice for a source, if it still fits the source.
    fn stored(&self, index: usize) -> Option<&str> {
        let mic = &self.mics[index];
        self.choices
            .iter()
            .rev()
            .find(|(name, _)| name == &mic.name)
            .map(|(_, value)| value.as_str())
            .filter(|value| valid_choice(value, mic.channels))
    }

    /// What the Inputs row shows for the target source.
    fn current(&self, index: usize) -> String {
        if let Some(value) = self.stored(index) {
            return value.to_string();
        }
        if Some(index) != self.used {
            return "off".into();
        }
        self.config
            .mic_channels
            .clone()
            .map(|value| value.replace('+', ","))
            .filter(|value| valid_choice(value, self.mics[index].channels))
            .unwrap_or_else(|| "all".into())
    }

    pub(crate) fn row(&self) -> Option<Item> {
        let index = self.target?;
        let mic = &self.mics[index];
        Some(Item {
            key: "clip:inputs".into(),
            group: "Capture".into(),
            name: format!(
                "Mic inputs · {}",
                row_label(&self.current(index), mic.channels)
            ),
            detail: mic.label().to_string(),
            keywords: "record microphone inputs channels interface multichannel".into(),
            favorite: false,
            target: Target::Mode(format!("{CLIP_INPUTS_MODE}{}", mic.name)),
        })
    }

    /// Arguments for the fixed recording rows.
    pub(crate) fn static_args(&self) -> Vec<String> {
        let Some(index) = self.target else {
            return Vec::new();
        };
        match self.stored(index) {
            Some(value) if Some(index) == self.used => {
                vec!["--mic-channels".into(), value.into()]
            }
            // Another source than the one NIRUCLIP would pick: name it.
            Some(value) if value != "off" => vec![
                "--mic-channels".into(),
                value.into(),
                "--mic".into(),
                self.mics[index].name.clone(),
            ],
            _ => Vec::new(),
        }
    }

    /// Arguments for a profile: its microphone keeps its own `mic`, and a
    /// profile that sets `mic_channels` itself is left alone.
    pub(crate) fn profile_args(&self, name: &str, audio: &str) -> Vec<String> {
        if matches!(audio, "none" | "desktop") {
            return Vec::new();
        }
        let profile = self.config.profiles.iter().find(|(n, _, _)| n == name);
        if profile.is_some_and(|(_, _, channels)| *channels) {
            return Vec::new();
        }
        let wanted = profile
            .and_then(|(_, mic, _)| mic.as_deref())
            .or(self.config.mic.as_deref());
        match self.resolve(wanted) {
            Some(index) if self.mics[index].multichannel => match self.stored(index) {
                Some(value) if value != "off" => vec!["--mic-channels".into(), value.into()],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// The rows of the input picker for `source`.
    pub(crate) fn choice_items(&self, source: &str) -> Vec<Item> {
        let Some(index) = self.mics.iter().position(|mic| mic.name == source) else {
            return vec![Item {
                key: "clip-inputs:gone".into(),
                group: "Inputs".into(),
                name: "The interface is not there".into(),
                detail: "Plug it in and open Capture again".into(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            }];
        };
        let mic = &self.mics[index];
        let n = mic.channels;
        let channel = |i: usize| mic.channel_names.get(i - 1).cloned().unwrap_or_default();
        let mut choices: Vec<(String, String)> = Vec::new();
        if Some(index) != self.used {
            let used = self
                .used
                .map_or("the default microphone", |i| self.mics[i].label());
            choices.push(("off".into(), format!("Record from {used} as usual")));
        }
        choices.push((
            "all".into(),
            format!("{n} channels, as the interface sends them"),
        ));
        choices.extend((1..=n).map(|i| (i.to_string(), format!("Mono · {}", channel(i)))));
        choices.extend((1..n).step_by(2).map(|i| {
            (
                format!("{i},{}", i + 1),
                format!("Stereo · {} + {}", channel(i), channel(i + 1)),
            )
        }));
        let current = self.current(index);
        choices
            .into_iter()
            .map(|(value, detail)| Item {
                key: format!("clip-inputs:{value}"),
                group: "Inputs".into(),
                name: choice_label(&value, n),
                detail: if value == current {
                    format!("Current · {detail}")
                } else {
                    detail
                }
                .trim_end_matches([' ', '·'])
                .to_string(),
                keywords: format!("{} microphone channel", mic.label()),
                favorite: false,
                target: Target::Mode(format!("{CLIP_SET_INPUTS}{value}\t{}", mic.name)),
            })
            .collect()
    }
}

/// The input picker for a source, built in a worker.
pub(crate) fn clip_input_items(source: &str) -> Vec<Item> {
    MicContext::new(list_mics(), &ClipConfig::load(), &load_inputs()).choice_items(source)
}

/// Fixed rows that record the microphone (and so take the chosen inputs).
fn item_records_mic(item: &Item) -> bool {
    matches!(&item.target, Target::Spawn { argv, .. } if argv.windows(2).any(|w| w[0] == "--audio" && w[1] == "mic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `niruclip --list-mics --json` with an 18-input interface (not the
    /// default) and the built-in microphone.
    fn fake_mics() -> Vec<Mic> {
        let names: Vec<String> = (0..18).map(|i| format!("AUX{i}")).collect();
        parse_mics(
            serde_json::json!([
                {"name": "alsa_input.pci-0000_00_1f.3.analog-stereo", "description": "Built-in Audio Analog Stereo",
                 "default": true, "channels": 2, "channel_names": ["FL", "FR"], "multichannel": false},
                {"name": "alsa_input.usb-BEHRINGER_UMC1820.multichannel-input", "description": "UMC1820 Multichannel",
                 "default": false, "channels": 18, "channel_names": names, "multichannel": true}
            ])
            .to_string()
            .as_bytes(),
        )
    }

    const UMC: &str = "alsa_input.usb-BEHRINGER_UMC1820.multichannel-input";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn older_niruclip_and_no_interface_change_nothing() {
        // 0.6.x: no channel fields.
        let old = parse_mics(br#"[{"name":"a","description":"A","default":true}]"#);
        assert_eq!(old.len(), 1);
        assert!(!old[0].multichannel);
        let context = MicContext::new(old, &ClipConfig::default(), &[]);
        assert!(context.row().is_none());
        assert!(context.static_args().is_empty());
        // Text output (no --json) or garbage.
        assert!(parse_mics(b"* a  A\n").is_empty());
        assert!(parse_mics(b"").is_empty());
        // A stereo source marked multichannel by mistake is not one.
        let stereo = parse_mics(br#"[{"name":"s","channels":2,"multichannel":true}]"#);
        assert!(!stereo[0].multichannel);
    }

    #[test]
    fn the_picker_offers_all_mono_and_adjacent_pairs() {
        let context = MicContext::new(fake_mics(), &ClipConfig::default(), &[]);
        let rows = context.choice_items(UMC);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        // Not the default source: "Not used" first, then all, 18 mono, 9 pairs.
        assert_eq!(rows.len(), 1 + 1 + 18 + 9);
        assert_eq!(names[0], "Not used");
        assert_eq!(names[1], "All 18 inputs");
        assert_eq!(names[2], "Input 1 (mono)");
        assert_eq!(names[19], "Input 18 (mono)");
        assert_eq!(names[20], "Inputs 1+2 (stereo)");
        assert_eq!(names[28], "Inputs 17+18 (stereo)");
        assert_eq!(rows[4].detail, "Mono · AUX2");
        assert!(rows[0].detail.starts_with("Current · Record from Built-in"));
        assert!(
            matches!(&rows[21].target, Target::Mode(m) if m == &format!("{CLIP_SET_INPUTS}3,4\t{UMC}"))
        );
        assert_eq!(context.choice_items("gone").len(), 1);
    }

    #[test]
    fn the_choice_is_per_source_and_reaches_the_recording() {
        let config = ClipConfig::default();
        // No choice yet: the row is there, recordings are unchanged.
        let context = MicContext::new(fake_mics(), &config, &[]);
        let row = context.row().unwrap();
        assert_eq!(row.name, "Mic inputs · not used");
        assert_eq!(row.detail, "UMC1820 Multichannel");
        assert!(context.static_args().is_empty());
        // Inputs 1+2 of the interface, which is not the default: name it.
        let chosen = [(UMC.to_string(), "1,2".to_string())];
        let context = MicContext::new(fake_mics(), &config, &chosen);
        assert_eq!(context.row().unwrap().name, "Mic inputs · 1+2 stereo");
        assert_eq!(
            context.static_args(),
            args(&["--mic-channels", "1,2", "--mic", UMC])
        );
        // A full-screen row gets it after its own arguments.
        let full = clip_static_items_with(&context.static_args())
            .into_iter()
            .find(|item| item.key == "clip:full")
            .unwrap();
        assert!(
            matches!(&full.target, Target::Spawn { argv, .. } if argv.ends_with(&args(&["--mic-channels", "1,2", "--mic", UMC])))
        );
        // Stop and export rows do not.
        assert!(clip_static_items_with(&context.static_args())
            .iter()
            .filter(|item| !item_records_mic(item))
            .all(|item| !matches!(&item.target, Target::Spawn { argv, .. } if argv.iter().any(|a| a == "--mic-channels"))));
        // When the interface is the configured mic, no --mic is needed.
        let config = ClipConfig::parse("mic = \"UMC1820\"\n");
        let mono = [(UMC.to_string(), "3".to_string())];
        let context = MicContext::new(fake_mics(), &config, &mono);
        assert_eq!(context.static_args(), args(&["--mic-channels", "3"]));
        assert_eq!(context.row().unwrap().name, "Mic inputs · 3 mono");
        // Without a choice it shows niruclip's own mic_channels, or all.
        let config = ClipConfig::parse("mic = \"umc1820\"\nmic_channels = \"5+6\"\n");
        let context = MicContext::new(fake_mics(), &config, &[]);
        assert!(context.row().unwrap().name.ends_with("5+6 stereo"));
        let config = ClipConfig::parse("mic = \"UMC1820\"\n");
        let context = MicContext::new(fake_mics(), &config, &[]);
        assert!(context.row().unwrap().name.ends_with("all 18"));
        // A choice that no longer fits (input 30) is ignored.
        let stale = [(UMC.to_string(), "30".to_string())];
        let context = MicContext::new(fake_mics(), &config, &stale);
        assert!(context.static_args().is_empty());
        // "Not used" on a source that is not the used one adds nothing.
        let off = [(UMC.to_string(), "off".to_string())];
        let context = MicContext::new(fake_mics(), &ClipConfig::default(), &off);
        assert!(context.static_args().is_empty());
    }

    #[test]
    fn profiles_keep_their_own_microphone() {
        let config = ClipConfig::parse(
            "[profiles.band]\nmic = \"UMC1820\"\n[profiles.fixed]\nmic = \"UMC1820\"\nmic_channels = \"7\"\n",
        );
        let chosen = [(UMC.to_string(), "1,2".to_string())];
        let context = MicContext::new(fake_mics(), &config, &chosen);
        assert_eq!(
            context.profile_args("band", "mic"),
            args(&["--mic-channels", "1,2"])
        );
        // Its own mic_channels wins; desktop audio has no microphone.
        assert!(context.profile_args("fixed", "mic").is_empty());
        assert!(context.profile_args("band", "desktop").is_empty());
        // A profile on the built-in mic is not touched.
        assert!(context.profile_args("screen-mic", "mic").is_empty());
    }

    #[test]
    fn choices_are_remembered_per_source() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("nirulauncher-inputs-{}", std::process::id()));
        std::env::set_var("XDG_STATE_HOME", &dir);
        assert!(!has_input_choices());
        save_input(UMC, "3");
        save_input("other", "all");
        save_input(UMC, "1,2");
        assert_eq!(
            load_inputs(),
            vec![
                ("other".to_string(), "all".to_string()),
                (UMC.to_string(), "1,2".to_string())
            ]
        );
        save_input("bad\tname", "3");
        assert_eq!(load_inputs().len(), 2);
        std::env::remove_var("XDG_STATE_HOME");
        let _ = std::fs::remove_dir_all(dir);
    }

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
