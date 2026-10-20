//! NIRULANCHER — native application and desktop-action launcher.
//!
//! Keyboard-driven, one process per invocation. No indexing daemon, no network
//! and no shell: applications are started from argv arrays parsed from their
//! desktop entries.

mod config;
mod desktop;
mod render;

use config::Config;
use render::Renderer;
use std::process::{Command, Stdio};
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
}

#[derive(Clone)]
enum Target {
    Spawn {
        argv: Vec<String>,
        terminal: bool,
        path: Option<std::path::PathBuf>,
    },
    Focus(Window),
    Confirm(Action),
}

#[derive(Clone)]
struct Item {
    /// Stable key for favourites and local usage.
    key: String,
    group: String,
    name: String,
    detail: String,
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
    confirm: Option<Action>,
    error: String,
    selection: Atom,
    activate: Atom,
}

fn intern(c: &RustConnection, name: &str) -> Result<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn set_utf8(c: &RustConnection, win: Window, atom: Atom, value: &str) -> Result<()> {
    let utf8 = intern(c, "UTF8_STRING")?;
    c.change_property8(PropMode::REPLACE, win, atom, utf8, value.as_bytes())?;
    Ok(())
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
        ("Suspend", "Lock, then suspend", "suspend"),
    ] {
        let mut parts = action.split(' ');
        let first = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();
        let mut argv = vec!["niructl".to_string(), first.to_string()];
        argv.extend(rest.into_iter().map(str::to_string));
        let group = if matches!(first, "lock" | "screensaver" | "reload" | "dnd" | "idle") {
            "System"
        } else {
            "Display"
        };
        items.push(Item {
            key: format!("action:{name}"),
            group: group.into(),
            name: name.into(),
            detail: detail.into(),
            favorite: false,
            target: Target::Spawn {
                argv,
                terminal: false,
                path: None,
            },
        });
    }
    for theme in ["niru-noir", "niru-nord", "niru-satie", "niru-carl-larsson"] {
        items.push(Item {
            key: format!("action:theme:{theme}"),
            group: "Appearance".into(),
            name: format!("Theme: {theme}"),
            detail: "Apply across WM, bar, saver and lock".into(),
            favorite: false,
            target: niructl(&["theme", theme]),
        });
    }
    for ws in 1..=9 {
        items.push(Item {
            key: format!("action:workspace:{ws}"),
            group: "Workspaces".into(),
            name: format!("Workspace {ws}"),
            detail: "Switch the focused monitor".into(),
            favorite: false,
            target: niructl(&["tag", &ws.to_string()]),
        });
    }
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
            favorite: false,
            target: Target::Confirm(action),
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

fn filtered(
    items: &[Item],
    query: &str,
    usage: &std::collections::HashMap<String, u64>,
) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<(bool, u8, u64, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let label = item.label().to_lowercase();
            let name = item.name.to_lowercase();
            let detail = item.detail.to_lowercase();
            let rank = if q.is_empty() || name == q || label == q {
                0
            } else if name.starts_with(&q) || label.starts_with(&q) {
                1
            } else if name.contains(&q) || label.contains(&q) {
                2
            } else if detail.contains(&q) {
                3
            } else {
                return None;
            };
            let last_used = usage.get(&item.key).copied().unwrap_or(0);
            Some((!item.favorite, rank, last_used, i))
        })
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    hits.into_iter().map(|(_, _, _, i)| i).collect()
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

impl Ui {
    fn new(mode: &str) -> Result<Self> {
        let cfg = Config::load()?;
        let apps = desktop::discover();
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        let mut items = Vec::new();
        if mode == "actions" {
            items.extend(action_items(&cfg));
            items.extend(window_items(&c, root));
            items.extend(app_items(apps));
        } else if mode == "windows" {
            items.extend(window_items(&c, root));
        } else {
            items.extend(app_items(apps));
        }
        for item in &mut items {
            item.favorite = cfg.favorites.iter().any(|favorite| {
                favorite == &item.key
                    || favorite.eq_ignore_ascii_case(&item.name)
                    || favorite == &item.label()
            });
        }
        if items.is_empty() {
            return Err("no launchable applications or actions found".into());
        }
        let (px, py) = point(&c, root);
        let monitor = monitor_at(&c, root, px, py);
        let rows = items.len().min(cfg.max_rows).max(1);
        let header = cfg.row_height + cfg.padding * 2;
        let footer = cfg.row_height;
        let width = cfg.width.min((monitor.w as i16).max(200) as u16);
        let height = (header + footer + rows as i16 * cfg.row_height)
            .min(monitor.h as i16)
            .max(header + footer + cfg.row_height);
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
        let filtered = filtered(&items, "", &usage);
        let mut ui = Self {
            c,
            root,
            win,
            gc,
            cfg,
            renderer,
            items,
            filtered,
            usage,
            query: String::new(),
            selected: 0,
            offset: 0,
            rows,
            width,
            height: height as u16,
            confirm: None,
            error: String::new(),
            selection,
            activate,
        };
        ui.draw()?;
        Ok(ui)
    }

    fn draw(&mut self) -> Result<()> {
        let pal = self.cfg.palette;
        let pad = self.cfg.padding;
        let row = self.cfg.row_height;
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
        let title = if self.confirm.is_some() {
            "Confirm"
        } else if self
            .items
            .iter()
            .any(|i| matches!(i.target, Target::Confirm(_)))
        {
            "Search apps and actions"
        } else {
            "Search applications"
        };
        let query = if self.query.is_empty() {
            title.to_string()
        } else {
            format!("{title}: {}", self.query)
        };
        let colour = if self.query.is_empty() {
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
            }
            let name_colour = pal.foreground;
            let detail_colour = pal.muted;
            let baseline = y + row / 2 + self.renderer.line_height / 2 - 2;
            let label = item.label();
            let name = truncate(
                &label,
                (self.width as i16 - pad * 2) / self.renderer.advance.max(1),
            );
            self.renderer
                .line(&self.c, self.gc, pad, baseline, &name, name_colour, bg)?;
            let detail_width = (self.width as i16 / 3).max(80) / self.renderer.advance.max(1);
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
                }
            )
        } else if !self.error.is_empty() {
            self.error.clone()
        } else {
            "Enter launch · Esc close · ↑↓ move".to_string()
        };
        let footer_colour = if self.confirm.is_some() {
            pal.warning
        } else if !self.error.is_empty() {
            pal.error
        } else {
            pal.muted
        };
        let fy = self.height as i16 - self.cfg.padding / 2 - 2;
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

    fn refilter(&mut self) {
        self.filtered = filtered(&self.items, &self.query, &self.usage);
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.offset = 0;
        self.confirm = None;
        self.error.clear();
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

    fn execute(&mut self, key: &str, target: Target) -> Result<bool> {
        match target {
            Target::Confirm(action) => {
                if self.confirm.is_some() {
                    let command = match action {
                        Action::Logout => "logout",
                        Action::Reboot => "reboot",
                        Action::Shutdown => "shutdown",
                    };
                    if let Err(e) =
                        spawn_command(&["niructl".into(), command.into()], None, false, &self.cfg)
                    {
                        self.error = e;
                        self.confirm = None;
                        self.draw()?;
                        return Ok(true);
                    }
                    record_usage(key);
                    self.usage = load_usage();
                    return Ok(false);
                }
                self.confirm = Some(action);
                self.draw()?;
                Ok(true)
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
        let Some(index) = self.filtered.get(self.selected).copied() else {
            return Ok(true);
        };
        let target = self.items[index].target.clone();
        let key = self.items[index].key.clone();
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
            let event = match self.c.poll_for_event()? {
                Some(event) => event,
                None => {
                    self.c.flush()?;
                    self.c.wait_for_event()?
                }
            };
            match event {
                Event::KeyPress(e) => {
                    let (key, shift) = key_text(&self.c, e.detail, e.state);
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
                    if !handled {
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
        let header = self.cfg.row_height + self.cfg.padding * 2;
        if y < header {
            return None;
        }
        let line = ((y - header) / self.cfg.row_height) as usize;
        if line >= self.rows {
            return None;
        }
        let index = self.offset + line;
        (index < self.filtered.len()).then_some(index)
    }
}

fn truncate(text: &str, max_chars: i16) -> String {
    text.chars().take(max_chars.max(1) as usize).collect()
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

/// Keyboard level: base, Shift, AltGr (MOD5), then Shift+AltGr.
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
        "--apps" | "--actions" | "--windows" => {
            let mode = match arg {
                "--actions" => "actions",
                "--windows" => "windows",
                _ => "apps",
            };
            match Ui::new(mode) {
                Ok(mut ui) => ui.run(),
                Err(e) => Err(e),
            }
        }
        "--help" => {
            println!(
                "Usage: nirulauncher [--apps|--actions|--windows|--check-config|--list|--version]"
            );
            Ok(())
        }
        _ => {
            eprintln!(
                "Usage: nirulauncher [--apps|--actions|--windows|--check-config|--list|--version]"
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
        assert_eq!(order, vec![1, 2, 0], "favourite, then most recently used");
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
