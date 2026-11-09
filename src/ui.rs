//! The launcher window: opening, drawing, keys and the event loop.

use crate::*;

pub(crate) struct Ui {
    pub(crate) c: RustConnection,
    pub(crate) root: Window,
    pub(crate) win: Window,
    pub(crate) gc: Gcontext,
    pub(crate) cfg: Config,
    pub(crate) renderer: Renderer,
    pub(crate) items: Vec<Item>,
    pub(crate) filtered: Vec<usize>,
    pub(crate) usage: usage::Usage,
    /// Rows right after the favourites that are shown as recent.
    pub(crate) recent: std::ops::Range<usize>,
    /// The marker for recent rows; ASCII when the font has no arrow glyph.
    pub(crate) recent_mark: char,
    pub(crate) query: String,
    pub(crate) selected: usize,
    pub(crate) offset: usize,
    pub(crate) rows: usize,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) row_height: i16,
    pub(crate) padding: i16,
    pub(crate) confirm: Option<Action>,
    pub(crate) error: String,
    pub(crate) selection: Atom,
    pub(crate) activate: Atom,
    pub(crate) lock_request: Atom,
    /// True when the window was opened for a web search (`--search`).
    pub(crate) web_mode: bool,
    /// The current view, so an ordered list can keep its intended order.
    pub(crate) mode: String,
    /// The effective providers: order, aliases and the configured default.
    pub(crate) catalog: websearch::Catalog,
    /// How many items existed before the mode added its own rows.
    pub(crate) plain_items: usize,
    /// The profile name while a layout is being saved, with the cursor.
    pub(crate) profile_name: Option<String>,
    /// The minutes field while an idle/lock timing value is being edited.
    pub(crate) number_edit: Option<NumberEdit>,
    /// Latest Wi-Fi state, plus one in-flight backend operation.
    pub(crate) wifi: Option<wifi::Snapshot>,
    pub(crate) wifi_rx: Option<Receiver<std::result::Result<wifi::Snapshot, String>>>,
    pub(crate) wifi_busy: Option<String>,
    pub(crate) wifi_secret: Option<WifiSecret>,
    pub(crate) wifi_connect_pending: bool,
    pub(crate) wired: Option<wifi::WiredSnapshot>,
    pub(crate) wired_rx: Option<Receiver<std::result::Result<wifi::WiredSnapshot, String>>>,
    pub(crate) wired_busy: Option<String>,
    pub(crate) wired_connect_pending: Option<String>,
    pub(crate) sound: Option<sound::Snapshot>,
    pub(crate) sound_rx: Option<Receiver<std::result::Result<sound::Snapshot, String>>>,
    pub(crate) sound_test_rx: Option<Receiver<std::result::Result<sound::TestResult, String>>>,
    pub(crate) sound_test: Option<sound::TestResult>,
    pub(crate) sound_busy: Option<String>,
    /// Provider rows of the current view that are still loading.
    pub(crate) catalog_rx: Option<Receiver<Vec<Item>>>,
    /// The view those rows belong to.
    pub(crate) catalog_for: String,
    /// Whether arriving rows get favourite marks (the opening view does).
    pub(crate) catalog_favorites: bool,
    /// The user moved the selection, so arriving rows must keep it.
    pub(crate) moved: bool,
    pub(crate) keymap: Keymap,
    pub(crate) compose: compose::BuiltinCompose,
    /// Enter was pressed while the rows it should act on were loading.
    pub(crate) enter_after_load: bool,
    /// Event and per-app sounds from `nirux sounds status --json`; `None`
    /// while the first read is running.
    pub(crate) events: Option<std::result::Result<eventsound::Status, String>>,
    pub(crate) events_rx: Option<Receiver<EventsReply>>,
    pub(crate) events_busy: Option<String>,
    pub(crate) sound_pick: Option<SoundPick>,
    pub(crate) events_suggestions: Option<Vec<String>>,
    /// Emoji skin tone, 0 (none) to 5, cycled with Tab in the picker.
    pub(crate) tone: u8,
    /// Noto Color Emoji, opened when the first emoji row is drawn.
    pub(crate) emoji_font: Option<Option<emoji_font::EmojiFont>>,
    /// Ctrl+R in the cheat sheet: the next key combination is looked up.
    pub(crate) capture: bool,
    /// Shift was held with the Enter that activates a row.
    pub(crate) enter_shift: bool,
    /// Ctrl was held with that Enter (a file row opens its folder).
    pub(crate) enter_ctrl: bool,
    /// File search (`--files`, `/ query`): the running search and its rows.
    pub(crate) files: crate::modes::files::State,
    /// Text to paste into the previous window once this one has closed.
    pub(crate) paste_after: Option<String>,
    pub(crate) closed: bool,
}

pub(crate) fn intern(c: &RustConnection, name: &str) -> Result<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

pub(crate) fn set_utf8(c: &RustConnection, win: Window, atom: Atom, value: &str) -> Result<()> {
    let utf8 = intern(c, "UTF8_STRING")?;
    c.change_property8(PropMode::REPLACE, win, atom, utf8, value.as_bytes())?;
    Ok(())
}

pub(crate) fn monitor_at(c: &RustConnection, root: Window, px: i16, py: i16) -> Rect {
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

pub(crate) fn point(c: &RustConnection, root: Window) -> (i16, i16) {
    c.query_pointer(root)
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| (r.root_x, r.root_y))
        .unwrap_or((0, 0))
}

pub(crate) fn wait_until_viewable(c: &RustConnection, win: Window) -> Result<()> {
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
    pub(crate) fn new(mode: &str) -> Result<Self> {
        let cfg = Config::load()?;
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        // The first paint shows the rows that need no provider; the provider
        // rows start loading right after it, so they do not compete with it.
        let catalog = view_catalog(mode, &cfg, &c, root);
        let has_job = catalog.job.is_some();
        let provider_job = catalog.job;
        let mut items = catalog.items;
        mark_favorites(&mut items, &cfg);
        if items.is_empty() && !has_job && !matches!(mode, "search" | "wifi" | "sound" | "files") {
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
        // A view whose rows come from providers cannot know its length yet.
        let rows = if matches!(
            mode,
            "settings" | "wifi" | "system-actions" | "sound" | "files"
        ) || (has_job && mode != "apps")
        {
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
        let lock_request = intern(&c, "_NIRULANCHER_LOCK")?;
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
        let usage = load_usage(&cfg);
        let keymap = Keymap::load(&c);
        let recent_mark = if renderer.covers('↺') { '↺' } else { '+' };
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
            wifi_connect_pending: false,
            wired: None,
            wired_rx: None,
            wired_busy: None,
            wired_connect_pending: None,
            sound: None,
            sound_rx: None,
            sound_test_rx: None,
            sound_test: None,
            sound_busy: None,
            catalog_rx: None,
            catalog_for: mode.to_string(),
            catalog_favorites: true,
            moved: false,
            keymap,
            compose: compose::BuiltinCompose::default(),
            enter_after_load: false,
            events: None,
            events_rx: None,
            events_busy: None,
            sound_pick: None,
            events_suggestions: None,
            tone: if matches!(mode, "emoji" | "apps" | "actions" | "system-actions") {
                emoji::load_tone()
            } else {
                0
            },
            emoji_font: None,
            capture: false,
            enter_shift: false,
            enter_ctrl: false,
            files: Default::default(),
            paste_after: None,
            closed: false,
            items,
            filtered,
            usage,
            recent: 0..0,
            recent_mark,
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
            lock_request,
            web_mode,
            mode: mode.to_string(),
            catalog,
        };
        if mode == "files" {
            crate::modes::files::prefetch(&ui.cfg.files);
        }
        ui.refilter();
        if mode == "wifi" {
            ui.enter_wifi()?;
        } else if mode == "wired" {
            ui.enter_wired()?;
        } else if mode == "sound" {
            ui.enter_sound()?;
        }
        ui.draw()?;
        ui.catalog_rx = provider_job.map(start_provider_job);
        Ok(ui)
    }

    pub(crate) fn draw(&mut self) -> Result<()> {
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
        } else if self.mode == "agent" {
            "Ask OpenCode"
        } else if self.mode == "keys" {
            "Keybindings"
        } else if self.mode == "emoji" {
            "Emoji and symbols"
        } else if self.mode == "colors" {
            "Colors"
        } else if self.mode == "files" {
            "Find files"
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
        } else if self.mode == "timing"
            || self
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
        } else if self.items.iter().all(|item| item.group == "Quick Settings") {
            "Quick Settings"
        } else if self.mode == "share-pair" {
            "Pair NIRU Share device"
        } else if self.mode.starts_with(DISCORD_MODE) {
            "Send to Discord?"
        } else if self.items.iter().all(|item| item.group == "NIRU Share") {
            "NIRU Share"
        } else if self.items.iter().all(|item| item.group == "Clipboard") {
            "Clipboard History"
        } else if self.items.iter().all(|item| item.group == "Tailscale") {
            "Tailscale"
        } else if self.mode == "wifi" {
            "Wi-Fi"
        } else if self.mode == "wired" {
            "Wired network"
        } else if self.mode == "sound" {
            "Sound"
        } else if self.mode == "clip" {
            "Capture"
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
            if self.mode == "agent" {
                "Type a short prompt…".to_string()
            } else if let Some(hint) = mode_hint(&self.mode, self.web_mode) {
                format!("{title} · {hint}")
            } else {
                title.to_string()
            }
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
            let item = self.items[index].clone();
            let item = &item;
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
            let heading = keys::is_heading(item);
            let name_colour = if heading { pal.accent } else { pal.foreground };
            let detail_colour = pal.muted;
            let baseline = y + row / 2 + self.renderer.line_height / 2 - 2;
            let icon = row_icon(item, self.tone);
            let icon_cols = if icon.is_some() {
                ((row - 6) / self.renderer.advance.max(1)) + 2
            } else {
                0
            };
            let gap = " ".repeat(icon_cols.max(0) as usize);
            // Icon rows and the grouped cheat sheet show the name alone.
            let name_only = icon.is_some() || self.mode == "keys";
            let label = if heading {
                item.name.clone()
            } else if self.mode == "keys" {
                format!("  {}", item.name)
            } else if icon.is_some() {
                format!("{gap}{}", item.name)
            } else if item.favorite {
                format!("★ {}", item.label())
            } else if (self.query.trim().is_empty() || self.files_query().is_some())
                && self.recent.contains(&(self.offset + line))
            {
                format!("{} {}", self.recent_mark, item.label())
            } else {
                item.label()
            };
            let usable = ((self.width as i16 - pad * 2) / self.renderer.advance.max(1)).max(1);
            let name_width = (usable * 3 / 5).max(1);
            let detail_width = usable.saturating_sub(name_width + 1).max(1);
            let name = truncate(&label, name_width);
            self.renderer
                .line(&self.c, self.gc, pad, baseline, &name, name_colour, bg)?;
            // Why the row is here: the matched letters are underlined, and a
            // row found only through its keywords or detail names the word.
            let typed = self
                .files_query()
                .unwrap_or_else(|| self.query.trim().to_string());
            if !typed.is_empty() && !self.web_mode && self.mode != "agent" {
                let found = highlight(item, &typed);
                let shown_label = if name_only {
                    item.name.clone()
                } else {
                    item.label()
                };
                let shift = label
                    .chars()
                    .count()
                    .saturating_sub(shown_label.chars().count());
                let found = if name_only {
                    // The group is not shown on these rows, so only name
                    // positions can be marked.
                    let group = item.label().chars().count() - item.name.chars().count();
                    Highlight {
                        positions: found
                            .positions
                            .into_iter()
                            .filter(|p| *p >= group)
                            .map(|p| p - group)
                            .collect(),
                        keyword: found.keyword,
                    }
                } else {
                    found
                };
                let shown = name.chars().count();
                let advance = self.renderer.advance;
                let thickness = (self.renderer.line_height / 14).max(1);
                for position in found.positions {
                    let column = position + shift;
                    // The last visible column may be the truncation mark.
                    if column + 1 >= shown && shown < label.chars().count() {
                        continue;
                    }
                    if column >= shown {
                        continue;
                    }
                    self.renderer.fill(
                        &self.c,
                        self.gc,
                        Rectangle {
                            x: pad + column as i16 * advance,
                            y: baseline + 2,
                            width: advance.max(1) as u16,
                            height: thickness as u16,
                        },
                        pal.foreground,
                    )?;
                }
                if let Some(word) = found.keyword {
                    let room = name_width - shown as i16 - 1;
                    if room >= 4 {
                        let hint = truncate(&word, room);
                        self.renderer.line(
                            &self.c,
                            self.gc,
                            pad + (shown as i16 + 1) * advance,
                            baseline,
                            &hint,
                            pal.disabled,
                            bg,
                        )?;
                    }
                }
            }
            if let Some(icon) = icon {
                self.draw_icon(icon, pad, y, row, bg)?;
            }
            let detail = truncate(&item.detail, detail_width);
            let dx =
                (self.width as i16 - pad - detail.chars().count() as i16 * self.renderer.advance)
                    .max(pad);
            self.renderer
                .line(&self.c, self.gc, dx, baseline, &detail, detail_colour, bg)?;
        }
        if self.filtered.is_empty() && self.no_match_state() {
            let width = ((self.width as i16 - pad * 2) / self.renderer.advance.max(1)).max(1);
            for (line, text) in no_match_lines(&self.query).iter().enumerate() {
                let y = header_h + line as i16 * row;
                let baseline = y + row / 2 + self.renderer.line_height / 2 - 2;
                self.renderer.line(
                    &self.c,
                    self.gc,
                    pad,
                    baseline,
                    &truncate(text, width),
                    pal.muted,
                    pal.background,
                )?;
            }
        }
        let footer = if let Some(action) = &self.confirm {
            format!(
                "Press Enter again to {} · Esc cancels",
                match action {
                    Action::Logout => "log out",
                    Action::Reboot => "reboot",
                    Action::Shutdown => "shut down",
                    Action::UpdateShutdown => "update and shut down",
                    Action::UpdateReboot => "update and restart",
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
        } else if self.files_query().is_some() {
            let keys = "Enter open · Ctrl+Enter folder · Shift+Enter copy path";
            if self.files.rx.is_some() {
                format!("Searching… · {keys}")
            } else if self.files.note.is_empty() {
                format!("{keys} · Esc close")
            } else {
                format!("{} · {keys}", self.files.note)
            }
        } else if let Some(secret) = &self.wifi_secret {
            if secret.hidden && secret.stage == 0 {
                "Type the hidden SSID · Enter continues · Esc cancels".to_string()
            } else {
                "Type the password · Enter connects · Esc cancels".to_string()
            }
        } else if let Some(busy) = &self.wifi_busy {
            busy.clone()
        } else if self.mode == "agent" {
            "Type a prompt · Enter starts OpenCode · Esc closes".to_string()
        } else if self.mode == "share-pair" {
            "Type: host token · Enter pairs · Esc closes".to_string()
        } else if self.mode == "wifi" {
            "Enter connects or acts · Ctrl+R rescan · Esc close".to_string()
        } else if self.mode == "wired" {
            "Enter connects or disconnects the wired interface · Esc close".to_string()
        } else if self.mode == "sound" && self.sound_pick.is_some() {
            "Enter chooses · type to filter · Esc back".to_string()
        } else if self.mode == "sound" {
            "Enter selects or acts · Esc close · ↑↓ move".to_string()
        } else if self.mode == "clip" {
            "Enter starts or stops capture · Esc close · ↑↓ move".to_string()
        } else if self.web_mode {
            "Enter opens search · Tab moves · Esc closes".to_string()
        } else if self.mode == "keys" && self.capture {
            "Press a key combination to see what it does · Esc cancels".to_string()
        } else if let Some(note) = (self.mode == "keys")
            .then(|| self.filtered.get(self.selected))
            .flatten()
            .and_then(|&i| keys::note(&self.items[i]))
        {
            note.to_string()
        } else if self.mode == "keys" {
            "Enter runs · Ctrl+R: what does a key do? · type words or keys · Esc close".to_string()
        } else if self.mode == "emoji" {
            format!(
                "Enter pastes · Shift+Enter copies · Tab skin tone: {} · Esc close",
                emoji::tone_name(self.tone)
            )
        } else if self.mode == "colors" {
            "Enter copies the hex value · Esc close".to_string()
        } else if self.mode == "settings" {
            "Enter opens · Esc close · ↑↓ move".to_string()
        } else if self.mode == "themes" {
            "Enter applies · Esc close · ↑↓ move".to_string()
        } else if self.mode == "timing"
            || self
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

    /// Draw a row's icon in the cell before its name.
    pub(crate) fn draw_icon(
        &mut self,
        icon: Icon,
        x: i16,
        y: i16,
        row: i16,
        bg: u32,
    ) -> Result<()> {
        let size = (row - 6).max(8) as u16;
        let top = y + (row - size as i16) / 2;
        let baseline = y + row / 2 + self.renderer.line_height / 2 - 2;
        match icon {
            Icon::Swatch(colour) => {
                let rect = |dx: i16, d: u16| Rectangle {
                    x: x + dx,
                    y: top + dx,
                    width: size - d,
                    height: size - d,
                };
                self.renderer
                    .fill(&self.c, self.gc, rect(0, 0), self.cfg.palette.muted)?;
                self.renderer.fill(&self.c, self.gc, rect(1, 2), colour)?;
            }
            Icon::Emoji(text) => {
                if self.emoji_font.is_none() {
                    self.emoji_font = Some(emoji_font::EmojiFont::open(
                        std::env::var("NIRU_EMOJI_FONT").ok().as_deref(),
                    ));
                }
                let picture = self
                    .emoji_font
                    .as_mut()
                    .and_then(|f| f.as_mut())
                    .and_then(|f| f.picture(&text, size));
                match picture {
                    Some(p) => self
                        .renderer
                        .image(&self.c, self.gc, x, top, p.size, &p.pixels, bg)?,
                    None => self.renderer.line(
                        &self.c,
                        self.gc,
                        x,
                        baseline,
                        &text,
                        self.cfg.palette.foreground,
                        bg,
                    )?,
                }
            }
            Icon::Glyph(text) => {
                let shown: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                let shown = if shown.is_empty() {
                    "␣".to_string()
                } else {
                    shown
                };
                self.renderer.line(
                    &self.c,
                    self.gc,
                    x + self.renderer.advance / 2,
                    baseline,
                    &shown,
                    self.cfg.palette.foreground,
                    bg,
                )?;
            }
        }
        Ok(())
    }

    /// Ctrl+R in the cheat sheet: show what the pressed combination does.
    pub(crate) fn captured(&mut self, mods: u32, sym: u32) -> Result<()> {
        self.capture = false;
        let label = keys::combo_label(mods, sym);
        let token = keys::token(mods, sym);
        let bound: Vec<String> = self
            .items
            .iter()
            .filter(|item| {
                item.keywords
                    .split(keys::NOTE)
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .any(|w| w == token)
            })
            .map(|item| item.name.clone())
            .collect();
        self.query = label.to_lowercase();
        self.refilter();
        self.error = if bound.is_empty() {
            format!("{label} is not bound: it is free")
        } else {
            format!("{label}: {}", bound.join(" · "))
        };
        self.draw()
    }

    /// Save the selected command row as a persistent action. The launcher only
    /// writes through `niructl action save`; it never edits actions.toml.
    pub(crate) fn save_selected_action(&mut self) -> Result<()> {
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
                forget_providers();
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

    pub(crate) fn toggle_favorite(&mut self) -> Result<()> {
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
    pub(crate) fn name_key(&mut self, key: u32) -> Result<()> {
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
                            forget_providers();
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
                if let Some(ch) = key_char(key) {
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
    pub(crate) fn number_key(&mut self, key: u32) -> Result<()> {
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
                                forget_providers();
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
    pub(crate) fn web_items(&self) -> Vec<Item> {
        web_rows(&self.query, &self.catalog)
    }

    pub(crate) fn refilter(&mut self) {
        self.confirm = None;
        self.error.clear();
        if self.mode == "wifi" {
            self.refresh_wifi_items();
            return;
        }
        if self.mode == "sound" && self.sound_pick.is_some() {
            self.refresh_sound_items();
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
        if let Some(query) = self.files_query() {
            self.files.request(&self.cfg.files, &query);
            self.show_file_rows();
            return;
        }
        if self.files.sent.is_some() {
            // The `/` was deleted: back to the view's own rows.
            self.files.reset();
        }
        let intent_rows = if matches!(self.mode.as_str(), "apps" | "actions" | "system-actions")
            && !self.query.trim().is_empty()
        {
            // An arithmetic query gets its result as the first row.
            calc_row(&self.query)
                .into_iter()
                .chain(intent_items(&self.query))
                .chain(emoji::colon_rows(&self.query))
                .collect()
        } else {
            Vec::new()
        };
        let base = if self.query.trim().is_empty()
            && matches!(self.mode.as_str(), "settings" | "timing" | "wifi")
        {
            // These views have a deliberate order: appearance first, then the
            // lock ladder. Alphabetical sorting would scramble both.
            Ranked {
                order: (0..self.items.len()).collect(),
                recent: 0..0,
            }
        } else {
            ranked_for_mode(&self.items, &self.query, &self.usage, &self.mode)
        };
        let first = self.items.len();
        let count = intent_rows.len();
        self.items.extend(intent_rows);
        self.recent = base.recent.start + count..base.recent.end + count;
        self.filtered = intent_first_order(first, count, base.order);
        // The loading row stays visible while typing; it is not a match.
        if let Some(pending) = self.items.iter().position(is_pending) {
            if !self.filtered.contains(&pending) {
                self.filtered.push(pending);
            }
        }
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        if self
            .filtered
            .get(self.selected)
            .is_some_and(|&i| keys::is_heading(&self.items[i]))
            && self.selected + 1 < self.filtered.len()
        {
            self.selected += 1;
        }
        self.offset = 0;
    }

    /// The file search query: the whole field in the file view, the text
    /// after `/` in the unified views. `None` when file search is off.
    pub(crate) fn files_query(&self) -> Option<String> {
        if !self.cfg.files.enabled && self.mode != "files" {
            return None;
        }
        if self.mode == "files" {
            return Some(self.query.trim().to_string());
        }
        if matches!(self.mode.as_str(), "apps" | "actions" | "system-actions") {
            return crate::modes::files::prefixed(&self.query).map(str::to_string);
        }
        None
    }

    /// Show the file rows of the last finished search after the view's own
    /// rows (which the file search hides).
    pub(crate) fn show_file_rows(&mut self) {
        self.items.truncate(self.plain_items);
        let first = self.items.len();
        self.items.extend(self.files.rows.iter().cloned());
        self.filtered = (first..self.items.len()).collect();
        self.recent = 0..self.files.recent.min(self.filtered.len());
        self.selected = 0;
        self.offset = 0;
    }

    /// A file search finished: show its rows, and act on Enter pressed while
    /// it was running.
    pub(crate) fn poll_files(&mut self) -> Result<bool> {
        if !self.files.poll() {
            return Ok(false);
        }
        if self.files_query().is_some() {
            self.show_file_rows();
            self.draw()?;
        }
        Ok(true)
    }

    pub(crate) fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let last = self.filtered.len() as i32 - 1;
        let mut next = (self.selected as i32 + delta).clamp(0, last);
        // Group headings in the cheat sheet are skipped.
        let step = if delta < 0 { -1 } else { 1 };
        while keys::is_heading(&self.items[self.filtered[next as usize]]) {
            let after = next + step;
            if !(0..=last).contains(&after) {
                if next == 0 && last > 0 {
                    next = 1;
                }
                break;
            }
            next = after;
        }
        self.selected = next as usize;
        self.moved = true;
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if self.selected >= self.offset + self.rows {
            self.offset = self.selected + 1 - self.rows;
        }
        self.confirm = None;
    }

    /// A search that found nothing in a view where the web is the next stop.
    pub(crate) fn no_match_state(&self) -> bool {
        !self.query.trim().is_empty()
            && !self.web_mode
            && self.catalog_rx.is_none()
            && !matches!(
                self.mode.as_str(),
                "agent" | "share-pair" | "wifi" | "wired" | "keys" | "emoji" | "colors" | "files"
            )
            && self.files_query().is_none()
    }

    /// Tab with no matches: keep the text and show the web-search engines in
    /// this window (Super+S cannot reach the window manager while the
    /// launcher holds the keyboard).
    pub(crate) fn switch_to_web_search(&mut self) -> Result<()> {
        self.catalog_rx = None;
        self.mode = "search".into();
        self.web_mode = true;
        self.number_edit = None;
        self.profile_name = None;
        self.recent = 0..0;
        self.plain_items = 0;
        self.items.clear();
        self.filtered.clear();
        self.selected = 0;
        self.offset = 0;
        self.refilter();
        self.draw()
    }

    /// Merge provider rows that finished loading. The selection stays on the
    /// same row when the user already moved it, and a pending confirmation
    /// survives only while that row is still the selected one.
    pub(crate) fn poll_catalog(&mut self) -> Result<bool> {
        let Some(rx) = self.catalog_rx.as_ref() else {
            return Ok(false);
        };
        let rows = match rx.try_recv() {
            Ok(rows) => rows,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.catalog_rx = None;
        if self.catalog_for != self.mode {
            return Ok(true);
        }
        let previous = self
            .filtered
            .get(self.selected)
            .map(|&index| self.items[index].key.clone());
        let confirm = self.confirm.take();
        let error = std::mem::take(&mut self.error);
        self.items.truncate(self.plain_items);
        self.items.retain(|item| !is_pending(item));
        self.items.extend(rows);
        if self.catalog_favorites {
            mark_favorites(&mut self.items, &self.cfg);
        }
        if self.items.is_empty() {
            self.items.push(Item {
                key: format!("empty:{}", self.mode),
                group: String::new(),
                name: "Nothing to show in this view".into(),
                detail: String::new(),
                keywords: String::new(),
                favorite: false,
                target: Target::Info,
            });
        }
        self.plain_items = self.items.len();
        let keep = self.moved || confirm.is_some();
        self.refilter();
        self.error = error;
        if keep {
            if let Some(position) = previous.as_ref().and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&index| &self.items[index].key == key)
            }) {
                self.selected = position;
                self.confirm = confirm;
            }
        } else {
            self.selected = 0;
            if self
                .filtered
                .first()
                .is_some_and(|&i| keys::is_heading(&self.items[i]))
                && self.filtered.len() > 1
            {
                self.selected = 1;
            }
        }
        self.reveal_selection();
        self.draw()?;
        Ok(true)
    }

    pub(crate) fn reveal_selection(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if self.selected >= self.offset + self.rows {
            self.offset = self.selected + 1 - self.rows;
        }
    }

    pub(crate) fn switch_mode(&mut self, mode: &str) -> Result<()> {
        if mode == "wifi" {
            return self.enter_wifi();
        }
        if mode == "wired" {
            return self.enter_wired();
        }
        if mode == "sound" {
            return self.enter_sound();
        }
        // The rows a settings sub-view shows are built in-process so choosing
        // one swaps the open launcher instead of racing a second one.
        let catalog = view_catalog(mode, &self.cfg, &self.c, self.root);
        if catalog.items.is_empty() && catalog.job.is_none() && mode != "files" {
            self.error = "Nothing to show in this view".into();
            self.draw()?;
            return Ok(());
        }
        self.catalog_rx = catalog.job.map(start_provider_job);
        self.catalog_for = mode.to_string();
        self.catalog_favorites = false;
        self.moved = false;
        self.enter_after_load = false;
        self.items = catalog.items;
        self.plain_items = self.items.len();
        self.web_mode = false;
        self.mode = mode.to_string();
        self.query.clear();
        self.number_edit = None;
        self.profile_name = None;
        self.confirm = None;
        self.filtered = filtered_for_mode(&self.items, "", &self.usage, &self.mode);
        self.recent = 0..0;
        self.selected = 0;
        self.offset = 0;
        self.error.clear();
        self.files.reset();
        if mode == "files" {
            crate::modes::files::prefetch(&self.cfg.files);
            self.refilter();
        }
        self.draw()
    }

    /// A link, opened with the same code path as every other spawn.
    pub(crate) fn execute(&mut self, key: &str, target: Target) -> Result<bool> {
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
        if self.mode == "wired" {
            if let Some(keep) = self.wired_execute(key)? {
                return Ok(keep);
            }
        }
        if self.mode == "sound" {
            if let Some(keep) = self.events_execute(key)? {
                return Ok(keep);
            }
        }
        if let Some(path) = key.strip_prefix(crate::modes::files::KEY_PREFIX) {
            let result = if self.enter_shift {
                copy_to_clipboard(path)
            } else {
                let target = if self.enter_ctrl {
                    crate::modes::files::parent_of(path)
                } else {
                    path.to_string()
                };
                spawn_command(&["xdg-open".to_string(), target], None, false, &self.cfg)
            };
            return match result {
                Ok(()) => Ok(false),
                Err(e) => {
                    self.error = e;
                    self.draw()?;
                    Ok(true)
                }
            };
        }
        if key.starts_with(emoji::KEY_PREFIX) {
            let Some(text) = emoji::with_tone(key, self.tone) else {
                return Ok(true);
            };
            emoji::record_recent(key);
            if self.enter_shift {
                return match copy_to_clipboard(&text) {
                    Ok(()) => Ok(false),
                    Err(e) => {
                        self.error = e;
                        self.draw()?;
                        Ok(true)
                    }
                };
            }
            self.paste_after = Some(text);
            return Ok(false);
        }
        match target {
            Target::RunKey(keys) => {
                // Close first: the binding may open the launcher again, and
                // an action on "the focused window" must find that window.
                self.close()?;
                let _ = self.c.get_input_focus()?.reply();
                if let Err(e) = spawn_command(
                    &["niructl".to_string(), "keys".into(), "run".into(), keys],
                    None,
                    false,
                    &self.cfg,
                ) {
                    eprintln!("nirulauncher: {e}");
                }
                Ok(false)
            }
            Target::Confirm(action) => {
                if self.confirm.is_some() {
                    let commands: Vec<Vec<String>> = match action {
                        Action::Logout => vec![vec!["niructl".into(), "logout".into()]],
                        Action::Reboot => vec![vec!["niructl".into(), "reboot".into()]],
                        Action::Shutdown => vec![vec!["niructl".into(), "shutdown".into()]],
                        Action::UpdateShutdown => vec![vec![
                            "niructl".into(),
                            "update-shutdown".into(),
                            "--yes".into(),
                        ]],
                        Action::UpdateReboot => vec![vec![
                            "niructl".into(),
                            "update-reboot".into(),
                            "--yes".into(),
                        ]],
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
                    self.usage = load_usage(&self.cfg);
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
            Target::Copy(text) => match copy_to_clipboard(&text) {
                Ok(()) => Ok(false),
                Err(e) => {
                    self.error = e;
                    self.draw()?;
                    Ok(true)
                }
            },
            Target::Terminal { argv, hold } => {
                match spawn_terminal_command(&argv, hold, &self.cfg) {
                    Ok(()) => {
                        record_usage(key);
                        self.usage = load_usage(&self.cfg);
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
            Target::Agent(prompt) => match spawn_agent(&prompt, &self.cfg) {
                Ok(()) => {
                    record_usage(key);
                    self.usage = load_usage(&self.cfg);
                    Ok(false)
                }
                Err(e) => {
                    self.error = e;
                    self.confirm = None;
                    self.draw()?;
                    Ok(true)
                }
            },
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
                self.usage = load_usage(&self.cfg);
                Ok(false)
            }
            Target::Spawn {
                argv,
                terminal,
                path,
            } => match spawn_command(&argv, path.as_deref(), terminal, &self.cfg) {
                Ok(()) => {
                    record_usage(key);
                    self.usage = load_usage(&self.cfg);
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

    pub(crate) fn activate(&mut self) -> Result<bool> {
        if self.mode == "share-pair" {
            let parts: Vec<&str> = self.query.split_whitespace().collect();
            if parts.len() != 2 {
                self.error = "Type: host token".into();
                self.draw()?;
                return Ok(true);
            }
            let result = Command::new("nirushare")
                .args(["trust", parts[0]])
                .env("NIRUSHARE_TOKEN", parts[1])
                .status();
            return match result {
                Ok(status) if status.success() => {
                    record_usage("share:pair:open");
                    self.usage = load_usage(&self.cfg);
                    Ok(false)
                }
                _ => {
                    self.error = "Pairing failed".into();
                    self.draw()?;
                    Ok(true)
                }
            };
        } else if self.mode == "agent" {
            let prompt = self.query.trim().to_string();
            if prompt.is_empty() {
                self.error = "Type a prompt first".into();
                self.draw()?;
                return Ok(true);
            }
            if !in_path("opencode") {
                self.error = "OpenCode is not installed".into();
                self.draw()?;
                return Ok(true);
            }
            return self.execute("agent:prompt", Target::Agent(prompt));
        }
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

    pub(crate) fn close(&mut self) -> Result<()> {
        if std::mem::replace(&mut self.closed, true) {
            return Ok(());
        }
        let _ = self.c.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.c.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self
            .c
            .set_selection_owner(x11rb::NONE, self.selection, x11rb::CURRENT_TIME);
        self.c.destroy_window(self.win)?;
        self.c.flush()?;
        Ok(())
    }

    pub(crate) fn run(&mut self) -> Result<()> {
        loop {
            if self.wifi_rx.is_some() && self.poll_wifi()? {
                continue;
            }
            if self.wired_rx.is_some() && self.poll_wired()? {
                continue;
            }
            if self.sound_rx.is_some() && self.poll_sound()? {
                continue;
            }
            if self.sound_test_rx.is_some() && self.poll_sound_test()? {
                continue;
            }
            if self.events_rx.is_some() && self.poll_events()? {
                continue;
            }
            if self.files.rx.is_some() && self.poll_files()? {
                if self.files.rx.is_none()
                    && self.catalog_rx.is_none()
                    && std::mem::take(&mut self.enter_after_load)
                    && !self.activate()?
                {
                    self.close()?;
                    return Ok(());
                }
                continue;
            }
            if self.files.partial && self.files.rx.is_none() && !crate::modes::files::walk_running()
            {
                // The walker has listed everything: search the whole list.
                self.files.again(&self.cfg.files);
                continue;
            }
            if self.catalog_rx.is_some() && self.poll_catalog()? {
                if self.catalog_rx.is_none()
                    && self.files.rx.is_none()
                    && std::mem::take(&mut self.enter_after_load)
                    && !self.activate()?
                {
                    self.close()?;
                    return Ok(());
                }
                continue;
            }
            let event = match self.c.poll_for_event()? {
                Some(event) => event,
                None => {
                    if self.wifi_rx.is_some()
                        || self.wired_rx.is_some()
                        || self.sound_rx.is_some()
                        || self.sound_test_rx.is_some()
                        || self.catalog_rx.is_some()
                        || self.events_rx.is_some()
                        || self.files.rx.is_some()
                        || self.files.partial
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
                    let shift = e.state.contains(KeyButMask::SHIFT);
                    let sym = self.keymap.symbol(e.detail, e.state);
                    if self.capture {
                        let mods = u32::from(e.state.bits()) & keys::MODS;
                        if keys::is_modifier(sym) {
                            continue;
                        }
                        if sym == 0xff1b && mods == 0 {
                            self.capture = false;
                            self.draw()?;
                            continue;
                        }
                        let pressed = keys::pressed_keysym(&self.keymap, e.detail);
                        self.captured(mods, pressed)?;
                        continue;
                    }
                    // Dead keys compose with the next key; function keys
                    // cancel a pending accent and keep their meaning.
                    let key = if compose::is_function(sym) {
                        self.compose.reset();
                        sym
                    } else {
                        match self.compose.feed(sym) {
                            compose::Compose::Pass => sym,
                            compose::Compose::Composed(ch) => compose::char_keysym(ch),
                            compose::Compose::Composing | compose::Compose::Cancelled => continue,
                        }
                    };
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
                    if e.state.contains(KeyButMask::CONTROL) && key == 0x72 && self.mode == "keys" {
                        self.capture = true;
                        self.draw()?;
                        continue;
                    }
                    if (key == 0xff09 || key == 0xfe20) && self.mode == "emoji" {
                        let back = shift || key == 0xfe20;
                        self.tone = if back {
                            (self.tone + 5) % 6
                        } else {
                            (self.tone + 1) % 6
                        };
                        emoji::save_tone(self.tone);
                        self.draw()?;
                        continue;
                    }
                    if e.state.contains(KeyButMask::CONTROL)
                        && matches!(key, 0x70 | 0x73)
                        && self.files_query().is_some()
                    {
                        self.error = "File rows cannot be pinned or saved".into();
                        self.draw()?;
                        continue;
                    }
                    if e.state.contains(KeyButMask::CONTROL)
                        && key == 0x63
                        && self.files_query().is_some()
                    {
                        // Ctrl+C copies the selected file's path, like Shift+Enter.
                        self.enter_shift = true;
                        self.enter_ctrl = false;
                        if !self.activate()? {
                            self.close()?;
                            return Ok(());
                        }
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
                            } else if self.sound_pick.is_some() {
                                self.set_sound_pick(None)?;
                            } else {
                                self.close()?;
                                return Ok(());
                            }
                            true
                        }
                        0xff0d | 0xff8d => {
                            self.enter_shift = shift;
                            self.enter_ctrl = e.state.contains(KeyButMask::CONTROL);
                            // Enter before the provider rows arrived acts on
                            // the best match once they are there; in file
                            // search, on the rows of the query as typed.
                            let waiting = (self.catalog_rx.is_some()
                                && self
                                    .filtered
                                    .get(self.selected)
                                    .map_or(true, |&index| is_pending(&self.items[index])))
                                || (self.files.rx.is_some() && self.files_query().is_some());
                            if waiting {
                                self.enter_after_load = true;
                            } else if !self.activate()? {
                                self.close()?;
                                return Ok(());
                            }
                            true
                        }
                        0xff08 => {
                            if e.state.contains(KeyButMask::CONTROL) {
                                delete_word(&mut self.query);
                            } else {
                                self.query.pop();
                            }
                            self.refilter();
                            self.draw()?;
                            true
                        }
                        0xff52 => {
                            self.move_selection(-1);
                            self.draw()?;
                            true
                        }
                        0xff54 => {
                            self.move_selection(1);
                            self.draw()?;
                            true
                        }
                        0xff55 => {
                            self.move_selection(-(self.rows.max(1) as i32));
                            self.draw()?;
                            true
                        }
                        0xff56 => {
                            self.move_selection(self.rows.max(1) as i32);
                            self.draw()?;
                            true
                        }
                        // Home and End jump to the first and last row.
                        0xff50 => {
                            self.move_selection(-(self.filtered.len() as i32));
                            self.draw()?;
                            true
                        }
                        0xff57 => {
                            self.move_selection(self.filtered.len() as i32);
                            self.draw()?;
                            true
                        }
                        0xff09 if self.filtered.is_empty() && self.no_match_state() => {
                            self.switch_to_web_search()?;
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
                        if let Some(ch) = key_char(key) {
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
                        if matches!(e.detail, 4 | 6) {
                            self.move_selection(-1);
                            self.draw()?;
                        } else if matches!(e.detail, 5 | 7) {
                            self.move_selection(1);
                            self.draw()?;
                        } else if let Some(line) = self.row_at(e.event_y) {
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
                Event::MappingNotify(e) if e.request != Mapping::POINTER => {
                    self.keymap = Keymap::load(&self.c);
                    self.compose.reset();
                }
                Event::ClientMessage(e) if e.type_ == self.lock_request => {
                    // The lock screen cannot release another client's active
                    // grab. Close this overlay so NIRULOCK can take over input.
                    self.close()?;
                    return Ok(());
                }
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

    pub(crate) fn row_at(&self, y: i16) -> Option<usize> {
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

/// What a row shows before its name: an emoji bitmap, a symbol drawn with
/// the text font, or a color swatch.
pub(crate) enum Icon {
    Emoji(String),
    Glyph(String),
    Swatch(u32),
}

pub(crate) fn row_icon(item: &Item, tone: u8) -> Option<Icon> {
    if item.key.starts_with(emoji::KEY_PREFIX) {
        let chars = emoji::with_tone(&item.key, tone)?;
        return Some(if emoji::is_text_symbol(&item.key) {
            Icon::Glyph(chars)
        } else {
            Icon::Emoji(chars)
        });
    }
    colors::swatch(&item.key).map(Icon::Swatch)
}

pub(crate) fn truncate(text: &str, max_chars: i16) -> String {
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

/// The empty-query hint after a view's title.
pub(crate) fn mode_hint(mode: &str, web: bool) -> Option<&'static str> {
    if web {
        return Some("type a phrase or an address");
    }
    Some(match mode {
        "apps" => "type an app, a keyword, 2+2 or / for files",
        "actions" => "type an action, window, app, 2+2 or / for files",
        "files" => "type a file or folder name · recent files first",
        "system-actions" => "type an action or 2+2",
        "windows" => "type a window title",
        "sound" => "type to filter devices and event sounds",
        "share-pair" | "wifi" | "wired" | "agent" => return None,
        _ => "type to filter",
    })
}

/// What an empty result list says instead of nothing.
pub(crate) fn no_match_lines(query: &str) -> [String; 2] {
    let query: String = query.trim().chars().take(48).collect();
    [
        format!("No matches. Press Super+S to search the web for ‘{query}’"),
        "Tab: switch view to web search · Backspace: edit · Esc: close".into(),
    ]
}

#[cfg(test)]
mod x11_tests {

    #[test]
    #[ignore = "requires a private X server without another window manager"]
    fn focus_waits_for_window_manager_mapping() {
        use crate::*;
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
        crate::wait_until_viewable(&client, win).unwrap();
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
}
