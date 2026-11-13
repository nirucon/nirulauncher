//! The Sound and microphone views, including event and per-app sounds.

use crate::*;

/// Keyboard-accessible input controls. Device IDs and state come from niructl;
/// no microphone or mute setting changes just by opening this view.
pub(crate) fn short_device(name: &str) -> String {
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

pub(crate) fn microphone_items() -> Vec<Item> {
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
    match providers::output("niructl", &["audio", "sources"]) {
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

/// The full sound tool: every device, program, camera, remembered setting
/// and the doctor that explains silence.
pub(crate) fn niruio_item() -> Item {
    Item {
        key: "sound:niruio".into(),
        group: "Audio".into(),
        name: "Open NIRUIO".into(),
        detail: "Outputs, inputs, programs, cameras, remembered settings and the sound doctor · Super+Shift+A"
            .into(),
        keywords: "sound audio niruio mixer doctor camera microphone silent muted".into(),
        favorite: false,
        target: crate::modes::actions::niruio_window(),
    }
}

pub(crate) fn sound_info(name: &str, detail: &str) -> Item {
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

pub(crate) fn sound_items(
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
        return vec![
            sound_info(
                "Audio unavailable",
                snapshot
                    .reason
                    .as_deref()
                    .unwrap_or("PipeWire is not responding"),
            ),
            niruio_item(),
        ];
    }
    let mut items = vec![niruio_item()];
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

pub(crate) fn event_row(key: String, name: String, detail: String, target: Target) -> Item {
    Item {
        key,
        group: "Event sounds".into(),
        name,
        detail,
        keywords: "sound sounds event".into(),
        favorite: false,
        target,
    }
}

/// The event-sound part of the Sound view.
pub(crate) fn event_sound_items(
    state: Option<&std::result::Result<eventsound::Status, String>>,
    busy: Option<&str>,
) -> Vec<Item> {
    let info = |name: &str, detail: &str| {
        vec![event_row(
            "events:info".into(),
            name.into(),
            detail.into(),
            Target::Info,
        )]
    };
    let status = match state {
        None => return info("Event sounds", "Loading…"),
        Some(Err(reason)) => return info("Event sounds unavailable", reason),
        Some(Ok(status)) => status,
    };
    let mut items = Vec::new();
    if let Some(busy) = busy {
        items.push(event_row(
            "events:busy".into(),
            busy.into(),
            "Working…".into(),
            Target::Info,
        ));
    }
    let onoff = |on: bool| if on { "on" } else { "off" };
    items.push(event_row(
        "events:global".into(),
        format!("All event sounds: {}", onoff(status.enabled)),
        format!(
            "Enter turns them {} · volume {}% (set in sounds.toml)",
            onoff(!status.enabled),
            status.volume
        ),
        Target::Info,
    ));
    for (category, events) in status.groups() {
        let category_on = match &category {
            Some((name, on)) => {
                items.push(event_row(
                    format!("events:category:{name}"),
                    format!("Category {name}: {}", onoff(*on)),
                    if status.enabled {
                        format!("Enter turns every {name} sound {}", onoff(!on))
                    } else {
                        "Every event sound is off".into()
                    },
                    Target::Info,
                ));
                *on
            }
            None => true,
        };
        for event in &events {
            let on = status.events.get(event);
            items.push(event_row(
                format!("events:toggle:{event}"),
                format!("Event {event}: {}", onoff(on)),
                if !status.enabled {
                    "Every event sound is off".into()
                } else if !category_on {
                    format!(
                        "Its category is off · Enter turns the {event} sound {}",
                        onoff(!on)
                    )
                } else {
                    format!("Enter turns the {event} sound {}", onoff(!on))
                },
                Target::Info,
            ));
            items.push(event_row(
                format!("events:test:{event}"),
                format!("Event {event}: test"),
                format!("Play the {event} sound now"),
                Target::Info,
            ));
        }
    }
    let mut apps = status.apps.clone();
    apps.sort_by(|a, b| a.app.cmp(&b.app));
    for app in &apps {
        let name = eventsound::display_name(&app.app);
        let sound = if app.sound.is_empty() {
            "default"
        } else {
            &app.sound
        };
        items.push(event_row(
            format!("events:app:{}", app.app),
            format!("App {name} → {sound}"),
            format!("{} · Enter chooses a sound", eventsound::state_text(app)),
            Target::Info,
        ));
        items.push(event_row(
            format!("events:app-test:{}", app.app),
            format!("App {name}: test"),
            format!("Play the notification sound for {name}"),
            Target::Info,
        ));
        items.push(event_row(
            format!("events:app-remove:{}", app.app),
            format!("App {name}: remove"),
            format!("{name} plays the normal notification sound again"),
            Target::Info,
        ));
    }
    items.push(event_row(
        "events:add".into(),
        "App: add an app…".into(),
        "Give an app its own notification sound".into(),
        Target::Info,
    ));
    items
}

/// The rows of a choice list inside the Sound view.
pub(crate) fn sound_pick_items(
    pick: &SoundPick,
    status: Option<&eventsound::Status>,
    suggestions: Option<&[String]>,
    query: &str,
) -> Vec<Item> {
    let row = |key: String, group: &str, name: String, detail: &str| Item {
        key,
        group: group.into(),
        name,
        detail: detail.into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    };
    match pick {
        SoundPick::Sound { app } => {
            let group = format!("Sound for {}", eventsound::display_name(app));
            let current = status
                .and_then(|status| status.apps.iter().find(|entry| &entry.app == app))
                .map(|entry| entry.sound.as_str())
                .unwrap_or("");
            let mark = |name: &str| if name == current { "current" } else { "" };
            let mut items = vec![
                row(
                    "pick:sound:default".into(),
                    &group,
                    "Default".into(),
                    if matches!(current, "default" | "notification") {
                        "current · the notification sound"
                    } else {
                        "The notification sound"
                    },
                ),
                row(
                    "pick:sound:off".into(),
                    &group,
                    "Off".into(),
                    if matches!(current, "off" | "none" | "silent") {
                        "current · silent"
                    } else {
                        "Silent"
                    },
                ),
            ];
            let names = status
                .map(|status| eventsound::sound_names(&eventsound::sound_dirs(status)))
                .unwrap_or_default();
            for name in names {
                let detail = mark(&name);
                items.push(row(format!("pick:sound:{name}"), &group, name, detail));
            }
            items
        }
        SoundPick::App => {
            let group = "Add app";
            let mut items = Vec::new();
            let typed = query.trim();
            if !typed.is_empty() {
                items.push(row(
                    "pick:typed".into(),
                    group,
                    format!("Use the name {typed}"),
                    if eventsound::valid_app(&eventsound::app_key(typed)) {
                        "Enter chooses its sound"
                    } else {
                        "Letters, digits, '.', '_' and '-' only"
                    },
                ));
            }
            match suggestions {
                None => items.push(row(
                    "pick:loading".into(),
                    group,
                    "Loading recent notification apps…".into(),
                    "…",
                )),
                Some(apps) => {
                    for app in apps {
                        items.push(row(
                            format!("pick:app:{app}"),
                            group,
                            app.clone(),
                            "Seen in recent notifications",
                        ));
                    }
                }
            }
            if items.is_empty() {
                items.push(row(
                    "pick:hint".into(),
                    group,
                    "Type an app name".into(),
                    "As the app names itself in notifications",
                ));
            }
            items
        }
    }
}

pub(crate) fn microphone_info(name: &str, detail: &str) -> Item {
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

impl Ui {
    /// Open the compact sound panel. It paints immediately, then loads the
    /// PipeWire snapshot in a worker so the launcher never waits on wpctl.
    pub(crate) fn enter_sound(&mut self) -> Result<()> {
        self.catalog_rx = None;
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
        self.sound_pick = None;
        self.refresh_sound_items();
        if self.events_rx.is_none() {
            self.start_events_job(None)?;
        }
        self.start_sound_job("Loading audio…".into())
    }

    pub(crate) fn refresh_sound_items(&mut self) {
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        if let Some(pick) = self.sound_pick.clone() {
            self.items = sound_pick_items(
                &pick,
                self.events.as_ref().and_then(|state| state.as_ref().ok()),
                self.events_suggestions.as_deref(),
                &self.query,
            );
        } else {
            let snapshot = self.sound.clone();
            let test = self.sound_test.clone();
            let busy = self.sound_busy.clone();
            self.items = sound_items(snapshot.as_ref(), test.as_ref(), busy.as_deref());
            self.items.extend(event_sound_items(
                self.events.as_ref(),
                self.events_busy.as_deref(),
            ));
        }
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
        self.reveal_selection();
    }

    /// Read the event sounds, or run one `nirux sounds` change and read them
    /// again, in a worker.
    pub(crate) fn start_events_job(&mut self, change: Option<(String, Vec<String>)>) -> Result<()> {
        if self.events_rx.is_some() {
            self.error = "Wait for the current sound change".into();
            return self.draw();
        }
        let (tx, rx) = mpsc::channel();
        self.events_rx = Some(rx);
        match change {
            Some((label, args)) => {
                self.events_busy = Some(label);
                thread::spawn(move || {
                    let (state, answer) = eventsound::change(args);
                    let _ = tx.send(EventsReply::State(state, Some(answer)));
                });
            }
            None => {
                thread::spawn(move || {
                    let _ = tx.send(EventsReply::State(eventsound::status(), None));
                });
            }
        }
        self.refresh_sound_items();
        self.draw()
    }

    pub(crate) fn poll_events(&mut self) -> Result<bool> {
        let Some(rx) = self.events_rx.as_ref() else {
            return Ok(false);
        };
        let reply = match rx.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                EventsReply::State(Err("the sound worker stopped".into()), None)
            }
        };
        self.events_rx = None;
        self.events_busy = None;
        match reply {
            EventsReply::State(state, answer) => {
                self.events = Some(state);
                match answer {
                    Some(Ok(text)) if !text.is_empty() => self.error = text,
                    Some(Err(error)) => self.error = error,
                    _ => {}
                }
            }
            EventsReply::Suggestions(apps) => self.events_suggestions = Some(apps),
        }
        if self.sound_pick == Some(SoundPick::App) && self.events_suggestions.is_none() {
            // A change was running when "Add app…" opened.
            return self.set_sound_pick(Some(SoundPick::App)).map(|_| true);
        }
        if self.mode == "sound" {
            self.refresh_sound_items();
            self.draw()?;
        }
        Ok(true)
    }

    /// Open or leave a choice list in the Sound view.
    pub(crate) fn set_sound_pick(&mut self, pick: Option<SoundPick>) -> Result<()> {
        self.sound_pick = pick;
        self.query.clear();
        self.selected = 0;
        self.offset = 0;
        self.moved = false;
        if self.sound_pick == Some(SoundPick::App) && self.events_suggestions.is_none() {
            let known = match &self.events {
                Some(Ok(status)) => status.apps.clone(),
                _ => Vec::new(),
            };
            if self.events_rx.is_none() {
                let (tx, rx) = mpsc::channel();
                self.events_rx = Some(rx);
                thread::spawn(move || {
                    let _ = tx.send(EventsReply::Suggestions(eventsound::history_apps(&known)));
                });
            }
        }
        self.refresh_sound_items();
        self.draw()
    }

    /// Handle an event-sound row. `None` means it was not one.
    pub(crate) fn events_execute(&mut self, key: &str) -> Result<Option<bool>> {
        let status = match &self.events {
            Some(Ok(status)) => Some(status.clone()),
            _ => None,
        };
        let change = |label: String, args: Vec<String>| Some((label, args));
        let job = if key == "events:global" {
            let on = !status.as_ref().is_some_and(|status| status.enabled);
            change(
                if on {
                    "Turning event sounds on…"
                } else {
                    "Turning event sounds off…"
                }
                .into(),
                eventsound::set_enabled(on),
            )
        } else if let Some(category) = key.strip_prefix("events:category:") {
            change(
                format!("Switching the {category} sounds…"),
                eventsound::toggle_event(category),
            )
        } else if let Some(event) = key.strip_prefix("events:toggle:") {
            change(
                format!("Switching the {event} sound…"),
                eventsound::toggle_event(event),
            )
        } else if let Some(event) = key.strip_prefix("events:test:") {
            change(
                format!("Playing the {event} sound…"),
                eventsound::test_event(event),
            )
        } else if let Some(app) = key.strip_prefix("events:app-test:") {
            change(
                format!("Playing the sound for {app}…"),
                eventsound::test_app(app),
            )
        } else if let Some(app) = key.strip_prefix("events:app-remove:") {
            change(format!("Removing {app}…"), eventsound::remove_app(app))
        } else if let Some(app) = key.strip_prefix("events:app:") {
            self.set_sound_pick(Some(SoundPick::Sound { app: app.into() }))?;
            return Ok(Some(true));
        } else if key == "events:add" {
            self.set_sound_pick(Some(SoundPick::App))?;
            return Ok(Some(true));
        } else if let Some(app) = key.strip_prefix("pick:app:") {
            self.set_sound_pick(Some(SoundPick::Sound { app: app.into() }))?;
            return Ok(Some(true));
        } else if key == "pick:typed" {
            let app = eventsound::app_key(&self.query);
            if !eventsound::valid_app(&app) {
                self.error = "App names may contain letters, digits, '.', '_' and '-'".into();
                self.draw()?;
                return Ok(Some(true));
            }
            self.set_sound_pick(Some(SoundPick::Sound { app }))?;
            return Ok(Some(true));
        } else if let Some(sound) = key.strip_prefix("pick:sound:") {
            let Some(SoundPick::Sound { app }) = self.sound_pick.clone() else {
                return Ok(Some(true));
            };
            self.sound_pick = None;
            self.query.clear();
            change(
                format!("Giving {app} the {sound} sound…"),
                eventsound::set_app(&app, sound),
            )
        } else {
            return Ok(None);
        };
        self.start_events_job(job)?;
        Ok(Some(true))
    }

    pub(crate) fn start_sound_job(&mut self, label: String) -> Result<()> {
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

    pub(crate) fn poll_sound(&mut self) -> Result<bool> {
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
            // Say why instead of leaving "Loading audio…" up for good.
            Err(error) => {
                self.sound = Some(sound::Snapshot {
                    available: false,
                    reason: Some(error.clone()),
                    ..Default::default()
                });
                self.error = error;
            }
        }
        self.refresh_sound_items();
        self.draw()?;
        Ok(true)
    }

    pub(crate) fn start_sound_test(&mut self, seconds: u64) -> Result<()> {
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

    pub(crate) fn poll_sound_test(&mut self) -> Result<bool> {
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
}

#[cfg(test)]
mod event_sound_tests {
    use crate::{event_sound_items, sound_pick_items, SoundPick};

    const FIXTURE: &str = r#"{"schema":1,"enabled":true,"volume":40,"config":"/nonexistent/nirux/sounds.toml","events":{"startup":true,"notification":true,"update":false,"shutdown":true},"exclude":["nplay"],"apps":[{"app":"thunderbird","sound":"mail","state":"ok","file":"/x/mail.ogg"},{"app":"signal","sound":"off","state":"silent","file":""}]}"#;

    #[test]
    fn the_sound_view_lists_events_and_app_sounds() {
        let status = crate::eventsound::parse(FIXTURE.as_bytes()).unwrap();
        let rows = event_sound_items(Some(&Ok(status)), None);
        let find = |key: &str| {
            rows.iter()
                .find(|row| row.key == key)
                .unwrap_or_else(|| panic!("missing {key}"))
        };
        assert_eq!(find("events:global").name, "All event sounds: on");
        assert!(find("events:global").detail.contains("volume 40%"));
        assert_eq!(find("events:toggle:update").name, "Event update: off");
        assert_eq!(find("events:test:startup").name, "Event startup: test");
        assert_eq!(
            find("events:app:thunderbird").name,
            "App Thunderbird → mail"
        );
        assert!(find("events:app:thunderbird").detail.starts_with("ok"));
        assert_eq!(find("events:app:signal").name, "App Signal → off");
        assert!(find("events:app:signal").detail.starts_with("silent"));
        assert_eq!(find("events:app-test:signal").name, "App Signal: test");
        assert_eq!(find("events:app-remove:signal").name, "App Signal: remove");
        assert_eq!(find("events:add").name, "App: add an app…");
        assert!(rows.iter().all(|row| row.group == "Event sounds"));
    }

    const FIXTURE_219: &str = r#"{"schema":1,"enabled":true,"volume":35,"config":"/nonexistent/sounds.toml","events":{"startup":true,"notification":true,"volume-up":true,"brightness-up":false,"power-plug":true},"categories":{"power":{"enabled":false,"events":["power-plug"]},"feedback":{"enabled":true,"events":["volume-up","brightness-up"]},"system":{"enabled":true,"events":["startup"]},"notifications":{"enabled":true,"events":["notification"]}},"exclude":[],"apps":[]}"#;

    #[test]
    fn the_sound_view_lists_every_category_and_event_nirux_reports() {
        let status = crate::eventsound::parse(FIXTURE_219.as_bytes()).unwrap();
        let rows = event_sound_items(Some(&Ok(status)), None);
        let keys: Vec<&str> = rows.iter().map(|row| row.key.as_str()).collect();
        let pos = |key: &str| {
            keys.iter()
                .position(|k| *k == key)
                .unwrap_or_else(|| panic!("missing {key}"))
        };
        assert!(pos("events:category:system") < pos("events:category:notifications"));
        assert!(pos("events:category:notifications") < pos("events:category:feedback"));
        assert!(pos("events:category:feedback") < pos("events:category:power"));
        assert!(pos("events:category:feedback") < pos("events:toggle:volume-up"));
        assert!(pos("events:toggle:volume-up") < pos("events:category:power"));
        let find = |key: &str| &rows[pos(key)];
        assert_eq!(find("events:category:power").name, "Category power: off");
        assert_eq!(
            find("events:toggle:brightness-up").name,
            "Event brightness-up: off"
        );
        assert_eq!(
            find("events:toggle:power-plug").name,
            "Event power-plug: on"
        );
        assert!(find("events:toggle:power-plug")
            .detail
            .starts_with("Its category is off"));
        assert_eq!(find("events:test:volume-up").name, "Event volume-up: test");
        assert!(
            !keys.contains(&"events:toggle:update"),
            "only reported events are shown"
        );
    }

    #[test]
    fn an_older_nirux_shows_its_four_events_without_categories() {
        let status = crate::eventsound::parse(FIXTURE.as_bytes()).unwrap();
        let rows = event_sound_items(Some(&Ok(status)), None);
        assert!(rows
            .iter()
            .all(|row| !row.key.starts_with("events:category:")));
        for event in ["startup", "notification", "update", "shutdown"] {
            assert!(rows
                .iter()
                .any(|row| row.key == format!("events:toggle:{event}")));
        }
    }

    #[test]
    fn an_unavailable_nirux_is_one_row() {
        let rows = event_sound_items(
            Some(&Err("This NIRUX has no sound status; update NIRUX".into())),
            None,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Event sounds unavailable");
        assert_eq!(event_sound_items(None, None)[0].detail, "Loading…");
    }

    #[test]
    fn choice_lists_offer_default_off_and_typed_names() {
        let status = crate::eventsound::parse(FIXTURE.as_bytes()).unwrap();
        let rows = sound_pick_items(
            &SoundPick::Sound {
                app: "signal".into(),
            },
            Some(&status),
            None,
            "",
        );
        assert_eq!(rows[0].key, "pick:sound:default");
        assert_eq!(rows[1].key, "pick:sound:off");
        assert!(rows[1].detail.starts_with("current"), "Signal is off");
        let rows = sound_pick_items(
            &SoundPick::App,
            Some(&status),
            Some(&["element".to_string()]),
            "bad name!",
        );
        assert_eq!(rows[0].key, "pick:typed");
        assert!(rows[0].detail.contains("only"), "an invalid name is marked");
        assert_eq!(rows[1].key, "pick:app:element");
    }
}

#[cfg(test)]
mod device_name_tests {

    #[test]
    fn long_device_names_are_shortened() {
        assert_eq!(
            crate::short_device(
                "Tiger Lake-LP Smart Sound Technology Audio Controller Digital Microphone"
            ),
            "Digital Microphone"
        );
        assert_eq!(
            crate::short_device(
                "Tiger Lake-LP Smart Sound Technology Audio Controller HDMI / DisplayPort 3 Output"
            ),
            "HDMI / DisplayPort 3 Output"
        );
    }
}
