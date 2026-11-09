//! The views. Each view paints its rows that need no provider at once and
//! loads the rest in a worker ([`view_catalog`]).

use crate::*;

pub(crate) mod actions;
pub(crate) mod apps;
pub(crate) mod audio;
pub(crate) mod clip;
pub(crate) mod colors;
pub(crate) mod emoji;
pub(crate) mod files;
pub(crate) mod health;
pub(crate) mod keys;
pub(crate) mod network;
pub(crate) mod settings;
pub(crate) mod web;

/// Provider rows for a view, built in a worker thread.
pub(crate) type ProviderJob = Box<dyn FnOnce() -> Vec<Item> + Send>;

/// What a view paints at once, plus the provider rows that follow.
pub(crate) struct ViewCatalog {
    pub(crate) items: Vec<Item>,
    pub(crate) job: Option<ProviderJob>,
}

/// A row that stands in for provider rows that are still loading.
pub(crate) fn pending_item(mode: &str) -> Item {
    let (group, name) = match mode {
        "actions" | "system-actions" => ("System", "Loading devices, displays and diagnostics…"),
        "apps" => ("Saved actions", "Loading saved actions…"),
        "health" => (
            "Health",
            "Checking NIRUX, audio, network, display and capture…",
        ),
        "clip" => ("Capture", "Loading capture profiles…"),
        "microphone" => ("Audio", "Loading microphones…"),
        "bluetooth" => ("Bluetooth", "Loading Bluetooth…"),
        "share" => ("NIRU Share", "Loading NIRU Share…"),
        m if m.starts_with(DISCORD_MODE) => ("Discord", "Preparing the preview…"),
        "tailscale" => ("Tailscale", "Loading Tailscale peers…"),
        "displays" => ("Displays", "Loading displays…"),
        "power" => ("Power", "Loading power profiles…"),
        "keys" => ("Shortcuts", "Loading keybindings…"),
        "themes" => ("Appearance", "Loading themes…"),
        "timing" => ("Timing", "Loading idle and lock timing…"),
        "input" => ("Input", "Loading input devices…"),
        _ => ("", "Loading…"),
    };
    Item {
        key: format!("pending:{mode}"),
        group: group.into(),
        name: name.into(),
        detail: "…".into(),
        keywords: String::new(),
        favorite: false,
        target: Target::Info,
    }
}

pub(crate) fn is_pending(item: &Item) -> bool {
    item.key.starts_with("pending:")
}

pub(crate) fn saved_action_rows() -> Vec<Item> {
    let mut items = Vec::new();
    append_saved_actions(&mut items, &load_saved_actions());
    items
}

/// The rows of a view: the ones that need no provider, and a job for the rest.
pub(crate) fn view_catalog(
    mode: &str,
    cfg: &Config,
    c: &RustConnection,
    root: Window,
) -> ViewCatalog {
    let job = |job: ProviderJob, mut items: Vec<Item>| {
        items.push(pending_item(mode));
        ViewCatalog {
            items,
            job: Some(job),
        }
    };
    let ready = |items: Vec<Item>| ViewCatalog { items, job: None };
    match mode {
        "clip" => job(Box::new(clip_provider_items), clip_static_items()),
        "actions" => {
            let mut items = action_static_items(cfg);
            items.extend(window_items(c, root));
            items.extend(app_items(desktop::discover()));
            job(
                Box::new(|| {
                    let mut items = action_provider_items();
                    items.extend(saved_action_rows());
                    items
                }),
                items,
            )
        }
        "system-actions" => job(
            Box::new(|| {
                let mut items = action_provider_items();
                items.extend(saved_action_rows());
                items
            }),
            action_static_items(cfg),
        ),
        "microphone" => job(Box::new(microphone_items), Vec::new()),
        "bluetooth" => job(Box::new(bluetooth_items), Vec::new()),
        "agent" => ready(agent_items()),
        "share" => job(Box::new(share_items), Vec::new()),
        "share-pair" => ready(share_pair_items()),
        m if m.starts_with(DISCORD_MODE) => {
            let name = m[DISCORD_MODE.len()..].to_string();
            job(Box::new(move || share_discord_items(&name)), Vec::new())
        }
        "quick" => ready(quick_items()),
        "clipboard" => ready(clipboard_items()),
        "tailscale" => job(Box::new(tailscale_items), Vec::new()),
        "displays" => job(Box::new(display_items), Vec::new()),
        "power" => job(Box::new(power_items), Vec::new()),
        "windows" => ready(window_items(c, root)),
        // Dedicated web search: the provider rows are the whole list and are
        // built from the query, including when it is empty.
        "search" | "wifi" | "wired" => ready(Vec::new()),
        "keys" => job(Box::new(keys_items), Vec::new()),
        "emoji" => ready(emoji_items()),
        "colors" => ready(colors_items()),
        // The rows come from the query in a worker; see `files`.
        "files" => ready(Vec::new()),
        "settings" => ready(settings_items()),
        "themes" => job(Box::new(theme_items), Vec::new()),
        "timing" => job(Box::new(timing_items), Vec::new()),
        "input" => job(Box::new(input_items), Vec::new()),
        "sound" => ready(sound_items(None, None, Some("Loading audio…"))),
        "health" => job(Box::new(health_provider_items), health_static_items()),
        _ => job(Box::new(saved_action_rows), app_items(desktop::discover())),
    }
}

pub(crate) fn start_provider_job(job: ProviderJob) -> Receiver<Vec<Item>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}
