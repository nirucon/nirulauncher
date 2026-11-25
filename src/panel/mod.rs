//! NIRUPANEL: a floating panel in the middle of the screen (`Super+Space`,
//! `nirulauncher --panel`). A retro logo, a search field over the apps with
//! their icons, and modules: today, the system, updates, what plays, quick
//! switches and power. Which modules show, and in which order, is
//! `[panel] modules` in `~/.config/nirulauncher/config.toml`.
//!
//! It is the launcher's own process and its own parts (desktop entries,
//! ranking, usage, spawning), not a second launcher. Every source is local
//! and the slow ones run on worker threads, so the first frame is drawn at
//! once and the modules fill in.
mod data;
mod icons;

use crate::config::Config;
use crate::render::Renderer;
use crate::*;
use data::Toggle;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::*;

/// The modules there are, in their default order.
pub(crate) const MODULES: [&str; 7] = [
    "apps", "today", "system", "updates", "media", "toggles", "power",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Power {
    Lock,
    Logout,
    Suspend,
    Reboot,
    Shutdown,
}

impl Power {
    const ALL: [Power; 5] = [
        Power::Lock,
        Power::Suspend,
        Power::Logout,
        Power::Reboot,
        Power::Shutdown,
    ];
    fn label(self) -> &'static str {
        match self {
            Power::Lock => "Lock",
            Power::Logout => "Log out",
            Power::Suspend => "Sleep",
            Power::Reboot => "Restart",
            Power::Shutdown => "Shut down",
        }
    }
    fn command(self) -> &'static str {
        match self {
            Power::Lock => "lock",
            Power::Logout => "logout",
            Power::Suspend => "suspend",
            Power::Reboot => "reboot",
            Power::Shutdown => "shutdown",
        }
    }
    /// Ends the session or the computer: asks once more.
    fn asks(self) -> bool {
        matches!(self, Power::Logout | Power::Reboot | Power::Shutdown)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    App(usize),
    Previous,
    PlayPause,
    Next,
    OpenPlayer,
    Toggle(Toggle),
    Power(Power),
    Update,
    Today,
    System,
}

/// What the workers send back.
enum Msg {
    Today(data::Today),
    Updates(data::Updates),
    Media(Option<data::Media>),
    Toggle(Toggle, Option<bool>),
    Icon(String, Option<icons::Picture>),
    System(data::System),
}

struct Panel {
    c: RustConnection,
    root: Window,
    win: Window,
    gc: Gcontext,
    cfg: Config,
    r: Renderer,
    w: u16,
    h: u16,
    keymap: Keymap,
    selection: Atom,
    activate: Atom,
    modules: Vec<String>,
    app_rows: usize,
    items: Vec<Item>,
    icon_names: HashMap<String, String>,
    usage: usage::Usage,
    query: String,
    shown: Vec<usize>,
    pictures: HashMap<String, Option<icons::Picture>>,
    pending: std::collections::HashSet<String>,
    icon_jobs: Sender<(String, String, u16)>,
    targets: Vec<Hit>,
    focus: usize,
    hover: Option<Hit>,
    hits: Vec<(Rectangle, Hit)>,
    confirm: Option<Power>,
    note: String,
    today: Option<data::Today>,
    updates: Option<data::Updates>,
    media: Option<data::Media>,
    media_known: bool,
    toggles: HashMap<Toggle, Option<bool>>,
    system: Option<data::System>,
    system_busy: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    last_tick: Instant,
    last_media: Instant,
    closed: bool,
}

/// `nirulauncher --panel`: open it, or close the one that is open.
pub(crate) fn run() -> Result<()> {
    let mut p = Panel::open()?;
    let result = p.run();
    p.close();
    result
}

fn iso_week(year: i32, yday: i32, wday: i32) -> u32 {
    // ISO 8601: weeks start on Monday and week 1 holds the first Thursday.
    let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = |y: i32| if leap(y) { 366 } else { 365 };
    let wd = (wday + 6).rem_euclid(7); // Monday 0
    let jan1 = (wd - yday).rem_euclid(7);
    // A year has 53 weeks when it starts on a Thursday, or on a Wednesday
    // in a leap year.
    let weeks = |y: i32, first: i32| {
        if first == 3 || (first == 2 && leap(y)) {
            53
        } else {
            52
        }
    };
    let week = (yday - wd + 10) / 7;
    if week < 1 {
        let prev_jan1 = (jan1 - days(year - 1)).rem_euclid(7);
        weeks(year - 1, prev_jan1) as u32
    } else if week > weeks(year, jan1) {
        1
    } else {
        week as u32
    }
}

/// The local time: (hour, minute, "ons 25 nov", week).
fn clock() -> (i32, i32, String, u32) {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: a valid time and out-parameter.
    unsafe { libc::localtime_r(&now, &mut tm) };
    const DAYS: [&str; 7] = ["sön", "mån", "tis", "ons", "tor", "fre", "lör"];
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "maj", "jun", "jul", "aug", "sep", "okt", "nov", "dec",
    ];
    let date = format!(
        "{} {} {}",
        DAYS[tm.tm_wday.clamp(0, 6) as usize],
        tm.tm_mday,
        MONTHS[tm.tm_mon.clamp(0, 11) as usize]
    );
    let week = iso_week(tm.tm_year + 1900, tm.tm_yday, tm.tm_wday);
    (tm.tm_hour, tm.tm_min, date, week)
}

/// The NIRUPANEL letters in a 5 x 7 pixel font.
fn glyph(ch: char) -> [&'static str; 7] {
    match ch {
        'N' => [
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
        'I' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ],
        'R' => [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        'U' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'P' => [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        'A' => [
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        'L' => [
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        _ => ["....."; 7],
    }
}

fn inside(r: Rectangle, x: i16, y: i16) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.width as i16 && y < r.y + r.height as i16
}

fn rect(x: i16, y: i16, w: i32, h: i32) -> Rectangle {
    Rectangle {
        x,
        y,
        width: w.clamp(0, i32::from(u16::MAX)) as u16,
        height: h.clamp(0, i32::from(u16::MAX)) as u16,
    }
}

impl Panel {
    fn open() -> Result<Self> {
        let cfg = Config::load()?;
        let (c, screen) = x11rb::connect(None)?;
        let root = c.setup().roots[screen].root;
        let selection = intern(&c, "_NIRUPANEL_SELECTION")?;
        let activate = intern(&c, "_NIRUPANEL_CLOSE")?;
        // A second Super+Space closes the panel that is open.
        let owner = c.get_selection_owner(selection)?.reply()?.owner;
        if owner != x11rb::NONE {
            let _ = c.send_event(
                false,
                owner,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, owner, activate, [1, 0, 0, 0, 0]),
            );
            c.flush()?;
            std::process::exit(0);
        }
        let (px, py) = point(&c, root);
        let monitor = monitor_at(&c, root, px, py);
        let (advance, line) = Renderer::measure(cfg.font_path.as_deref(), cfg.font_size)?;
        let w = (i32::from(advance) * 128)
            .clamp(860, 1320)
            .min(i32::from(monitor.w) - i32::from(line) * 2) as u16;
        let modules = cfg.panel.modules.clone();
        let mut app_rows = cfg.panel.app_rows;
        // Tall enough for every module, shorter rows of apps on a small screen.
        let mut h = Self::height_for(&modules, app_rows, line, advance, w);
        while h > i32::from(monitor.h) - i32::from(line) * 2 && app_rows > 1 {
            app_rows -= 1;
            h = Self::height_for(&modules, app_rows, line, advance, w);
        }
        let h = h.min(i32::from(monitor.h) - 8).max(200) as u16;
        let x = monitor.x + (monitor.w as i16 - w as i16) / 2;
        let y = monitor.y + (monitor.h as i16 - h as i16) / 2;
        let win = c.generate_id()?;
        let gc = c.generate_id()?;
        let r = Renderer::new(&c, root, cfg.font_path.as_deref(), cfg.font_size, w, h)?;
        c.create_window(
            0,
            win,
            root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(cfg.palette.background)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::KEY_PRESS
                        | EventMask::BUTTON_PRESS
                        | EventMask::POINTER_MOTION,
                ),
        )?;
        c.create_gc(gc, win, &CreateGCAux::new())?;
        set_utf8(&c, win, intern(&c, "_NET_WM_NAME")?, "NIRUPANEL")?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"NIRUPANEL",
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirupanel\0NIRUPANEL\0",
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
        c.set_selection_owner(win, selection, x11rb::CURRENT_TIME)?;
        c.map_window(win)?;
        wait_until_viewable(&c, win)?;
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
            EventMask::BUTTON_PRESS | EventMask::POINTER_MOTION,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        );
        c.set_input_focus(InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME)?;
        c.flush()?;

        let apps = desktop::discover();
        let icon_names = apps
            .iter()
            .map(|a| (a.id.clone(), a.icon.clone()))
            .collect();
        let mut items = app_items(apps);
        mark_favorites(&mut items, &cfg);
        let usage = load_usage(&cfg);
        let (tx, rx) = channel();
        // Two workers draw the icons; each answers on the panel's channel.
        let (icon_jobs, jobs) = channel::<(String, String, u16)>();
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
        for _ in 0..2 {
            let (jobs, tx) = (jobs.clone(), tx.clone());
            std::thread::spawn(move || loop {
                let job = jobs.lock().ok().and_then(|j| j.recv().ok());
                let Some((key, icon, size)) = job else { break };
                let t = Instant::now();
                let picture = icons::load(&icon, size);
                if std::env::var_os("NIRUPANEL_DEBUG").is_some() && t.elapsed().as_millis() > 40 {
                    eprintln!("nirupanel: icon {icon} took {} ms", t.elapsed().as_millis());
                }
                if tx.send(Msg::Icon(key, picture)).is_err() {
                    break;
                }
            });
        }
        let keymap = Keymap::load(&c);
        let mut p = Self {
            c,
            root,
            win,
            gc,
            cfg,
            r,
            w,
            h,
            keymap,
            selection,
            activate,
            modules,
            app_rows,
            items,
            icon_names,
            usage,
            query: String::new(),
            shown: Vec::new(),
            pictures: HashMap::new(),
            pending: std::collections::HashSet::new(),
            icon_jobs,
            targets: Vec::new(),
            focus: 0,
            hover: None,
            hits: Vec::new(),
            confirm: None,
            note: String::new(),
            today: None,
            updates: None,
            media: None,
            media_known: false,
            toggles: HashMap::new(),
            system: None,
            system_busy: false,
            tx,
            rx,
            last_tick: Instant::now(),
            last_media: Instant::now(),
            closed: false,
        };
        p.refilter();
        let t = Instant::now();
        p.draw()?;
        if std::env::var_os("NIRUPANEL_DEBUG").is_some() {
            eprintln!("nirupanel: first frame in {} ms", t.elapsed().as_millis());
        }
        p.start_workers();
        Ok(p)
    }

    fn has(&self, module: &str) -> bool {
        self.modules.iter().any(|m| m == module)
    }

    /// Cards in a row for the panel's width.
    fn card_columns(w: u16, advance: i16, cards: usize) -> usize {
        let fit = (i32::from(w) / (i32::from(advance) * 26)).max(1) as usize;
        fit.min(cards).max(1)
    }

    fn height_for(modules: &[String], app_rows: usize, line: i16, advance: i16, w: u16) -> i32 {
        let lh = i32::from(line);
        let pad = lh;
        let mut h = pad + Self::logo_height(lh) + pad / 2 + lh * 2 + pad;
        let has = |m: &str| modules.iter().any(|x| x == m);
        if has("apps") {
            h += Self::tile_height(lh) * app_rows as i32 + pad;
        }
        let cards = ["today", "system", "updates", "media"]
            .iter()
            .filter(|m| has(m))
            .count();
        if cards > 0 {
            let cols = Self::card_columns(w, advance, cards);
            let rows = cards.div_ceil(cols) as i32;
            h += rows * (Self::card_height(lh) + pad / 2) + pad / 2;
        }
        for m in ["toggles", "power"] {
            if has(m) {
                h += lh * 2 + pad / 2;
            }
        }
        h + lh + pad
    }

    fn logo_height(lh: i32) -> i32 {
        (lh / 5).max(2) * 7 + 6
    }
    fn tile_height(lh: i32) -> i32 {
        Self::icon_size(lh) + lh + lh / 2 + 8
    }
    fn icon_size(lh: i32) -> i32 {
        (lh * 2 + lh / 2).clamp(32, 64)
    }
    fn card_height(lh: i32) -> i32 {
        lh * 6 + lh / 2
    }

    fn start_workers(&mut self) {
        if self.has("today") {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Today(data::today()));
            });
        }
        if self.has("updates") {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Updates(data::updates()));
            });
        }
        if self.has("media") {
            self.poll_media();
        }
        if self.has("system") {
            self.poll_system(Duration::from_millis(250));
        }
        if self.has("toggles") {
            for t in Toggle::ALL {
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::Toggle(t, t.state()));
                });
            }
        }
    }

    fn poll_system(&mut self, over: Duration) {
        if !self.has("system") || std::mem::replace(&mut self.system_busy, true) {
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::System(data::system(over)));
        });
    }

    fn poll_media(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Media(data::media()));
        });
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_string();
        let order = ranked_for_mode(&self.items, &q, &self.usage, "apps").order;
        self.shown = order;
        self.focus = 0;
        self.confirm = None;
    }

    /// The icon of an app: `Some(true)` drawn, `Some(false)` none (a
    /// letter tile), `None` still being drawn. Drawing an SVG can take a
    /// while the first time, so it happens on a worker and the panel shows
    /// at once; the icons come in as they are ready (and are cached).
    fn picture(&mut self, key: &str, size: u16) -> Option<bool> {
        if let Some(p) = self.pictures.get(key) {
            return Some(p.is_some());
        }
        if self.pending.insert(key.to_string()) {
            let icon = self.icon_names.get(key).cloned().unwrap_or_default();
            let jobs = self.icon_jobs.clone();
            let _ = jobs.send((key.to_string(), icon, size));
        }
        None
    }

    fn close(&mut self) {
        if std::mem::replace(&mut self.closed, true) {
            return;
        }
        let _ = self.c.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.c.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self
            .c
            .set_selection_owner(x11rb::NONE, self.selection, x11rb::CURRENT_TIME);
        let _ = self.c.destroy_window(self.win);
        let _ = self.c.flush();
    }

    // ------------------------------------------------------------- drawing

    fn fill(&self, r: Rectangle, colour: u32) -> Result<()> {
        self.r.fill(&self.c, self.gc, r, colour)
    }

    fn text(
        &mut self,
        x: i16,
        baseline: i16,
        text: &str,
        colour: u32,
        bg: u32,
        max: i32,
    ) -> Result<()> {
        let chars = (max / i32::from(self.r.advance).max(1)).max(1) as i16;
        let t = truncate(text, chars);
        self.r.line(&self.c, self.gc, x, baseline, &t, colour, bg)
    }

    fn baseline(&self, top: i16, height: i32) -> i16 {
        top + ((height - i32::from(self.r.line_height)) / 2) as i16 + (self.r.line_height * 4 / 5)
    }

    fn button(&mut self, r: Rectangle, label: &str, hit: Hit, on: Option<bool>) -> Result<()> {
        let pal = self.cfg.palette;
        let focused = self.targets.get(self.focus) == Some(&hit);
        let hovered = self.hover == Some(hit);
        let bg = match on {
            Some(true) => pal.accent,
            _ if focused || hovered => pal.selected,
            _ => pal.surface,
        };
        self.fill(r, if focused { pal.foreground } else { pal.border })?;
        self.fill(
            rect(
                r.x + 1,
                r.y + 1,
                i32::from(r.width) - 2,
                i32::from(r.height) - 2,
            ),
            bg,
        )?;
        let fg = match on {
            Some(true) => pal.background,
            Some(false) => pal.muted,
            None => pal.foreground,
        };
        let tw = label.chars().count() as i32 * i32::from(self.r.advance);
        let x = r.x + ((i32::from(r.width) - tw) / 2).max(4) as i16;
        let b = self.baseline(r.y, i32::from(r.height));
        self.text(x, b, label, fg, bg, i32::from(r.width) - 8)?;
        self.hits.push((r, hit));
        self.targets.push(hit);
        Ok(())
    }

    fn logo(&self, x: i16, y: i16, px: i16) -> Result<i16> {
        let pal = self.cfg.palette;
        let mut cx = x;
        for (k, ch) in "NIRUPANEL".chars().enumerate() {
            // The first half in the foreground, PANEL in the accent's light.
            let colour = if k < 4 { pal.foreground } else { pal.warning };
            for (row, line) in glyph(ch).iter().enumerate() {
                for (col, b) in line.bytes().enumerate() {
                    if b == b'#' {
                        self.fill(
                            rect(
                                cx + col as i16 * px,
                                y + row as i16 * px,
                                i32::from(px),
                                i32::from(px),
                            ),
                            colour,
                        )?;
                    }
                }
            }
            cx += px * 7;
        }
        // Four stripes under it, as on a home computer of the eighties.
        let stripes = [pal.error, pal.warning, pal.success, pal.accent];
        let sw = cx - x - px;
        for (i, c) in stripes.iter().enumerate() {
            self.fill(rect(x, y + px * 7 + 2 + i as i16 * 2, i32::from(sw), 1), *c)?;
        }
        Ok(cx)
    }

    fn card(&mut self, r: Rectangle, title: &str) -> Result<(i16, i16)> {
        let pal = self.cfg.palette;
        self.fill(r, pal.border)?;
        self.fill(
            rect(
                r.x + 1,
                r.y + 1,
                i32::from(r.width) - 2,
                i32::from(r.height) - 2,
            ),
            pal.surface,
        )?;
        let lh = self.r.line_height;
        self.fill(
            rect(r.x + 1, r.y + 1, i32::from(r.width) - 2, i32::from(lh) + 6),
            pal.selected,
        )?;
        let b = self.baseline(r.y + 1, i32::from(lh) + 6);
        self.text(
            r.x + 8,
            b,
            title,
            pal.warning,
            pal.selected,
            i32::from(r.width) - 16,
        )?;
        Ok((r.x + 8, r.y + lh + 10))
    }

    fn lines(&mut self, x: i16, y: i16, w: i32, lines: &[(String, u32)]) -> Result<()> {
        let lh = self.r.line_height + 2;
        let bg = self.cfg.palette.surface;
        for (i, (t, c)) in lines.iter().enumerate() {
            let b = self.baseline(y + i as i16 * lh, i32::from(lh));
            self.text(x, b, t, *c, bg, w)?;
        }
        Ok(())
    }

    fn meter(&self, x: i16, y: i16, w: i32, share: f64, colour: u32) -> Result<()> {
        let pal = self.cfg.palette;
        let h = (i32::from(self.r.line_height) / 3).max(3);
        self.fill(rect(x, y, w, h), pal.border)?;
        let filled = (f64::from(w) * share.clamp(0.0, 1.0)).round() as i32;
        self.fill(rect(x, y, filled, h), colour)
    }

    fn draw(&mut self) -> Result<()> {
        let pal = self.cfg.palette;
        self.hits.clear();
        self.targets.clear();
        let (w, h) = (i32::from(self.w), i32::from(self.h));
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance);
        let pad = lh;
        self.fill(rect(0, 0, w, h), pal.border)?;
        self.fill(rect(1, 1, w - 2, h - 2), pal.background)?;
        // Header: the logo, the clock and the date.
        let px = (lh / 5).max(2) as i16;
        let top = pad as i16;
        self.logo(pad as i16, top, px)?;
        let (hh, mm, date, week) = clock();
        let right = format!("{date} · v{week}   {hh:02}:{mm:02}");
        let rw = right.chars().count() as i32 * adv;
        let b = self.baseline(top, i32::from(px) * 7);
        self.text(
            (w - pad - rw) as i16,
            b,
            &right,
            pal.foreground,
            pal.background,
            rw + adv,
        )?;
        let mut y = top as i32 + Self::logo_height(lh) + pad / 2;
        // The search field.
        let field = rect(pad as i16, y as i16, w - pad * 2, lh * 2);
        self.fill(field, pal.border)?;
        self.fill(
            rect(
                field.x + 1,
                field.y + 1,
                i32::from(field.width) - 2,
                i32::from(field.height) - 2,
            ),
            pal.surface,
        )?;
        let b = self.baseline(field.y, i32::from(field.height));
        if self.query.is_empty() {
            let hint = if self.has("apps") {
                "Type to find an app · Tab moves · Esc closes"
            } else {
                "Tab moves · Enter chooses · Esc closes"
            };
            self.text(field.x + 10, b, "›", pal.warning, pal.surface, adv * 2)?;
            self.text(
                field.x + 10 + adv as i16 * 2,
                b,
                hint,
                pal.muted,
                pal.surface,
                i32::from(field.width) - 20,
            )?;
        } else {
            let shown = format!("› {}▏", self.query);
            self.text(
                field.x + 10,
                b,
                &shown,
                pal.foreground,
                pal.surface,
                i32::from(field.width) - 20,
            )?;
        }
        y += lh * 2 + pad;
        let modules = self.modules.clone();
        let mut cards_done = false;
        for m in &modules {
            match m.as_str() {
                "apps" => y = self.draw_apps(y, pad)?,
                "today" | "system" | "updates" | "media" if !cards_done => {
                    cards_done = true;
                    y = self.draw_cards(y, pad)?;
                }
                "toggles" => y = self.draw_toggles(y, pad)?,
                "power" => y = self.draw_power(y, pad)?,
                _ => {}
            }
        }
        // The footer: a notice, or the keys.
        let footer = if !self.note.is_empty() {
            (self.note.clone(), pal.warning)
        } else if let Some(p) = self.confirm {
            (
                format!(
                    "{}? Enter or a click again to confirm, Esc to cancel",
                    p.label()
                ),
                pal.warning,
            )
        } else {
            (
                "Enter opens · arrows and Tab move · Esc closes · Super+Space again closes".into(),
                pal.muted,
            )
        };
        let b = self.baseline((h - lh - pad / 2) as i16, lh);
        self.text(
            pad as i16,
            b,
            &footer.0,
            footer.1,
            pal.background,
            w - pad * 2,
        )?;
        if self.focus >= self.targets.len() {
            self.focus = 0;
        }
        self.r.present(&self.c, self.gc, self.win, self.w)?;
        self.c.flush()?;
        Ok(())
    }

    fn draw_apps(&mut self, y: i32, pad: i32) -> Result<i32> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance);
        let w = i32::from(self.w);
        let th = Self::tile_height(lh);
        let isz = Self::icon_size(lh);
        let tw = (isz + lh * 2).max(adv * 11);
        let cols = ((w - pad * 2) / tw).max(1) as usize;
        let tw = (w - pad * 2) / cols as i32;
        let count = (cols * self.app_rows).min(self.shown.len());
        if count == 0 {
            let b = self.baseline(y as i16, lh * 2);
            self.text(
                pad as i16,
                b,
                "No app matches. Enter searches the launcher.",
                pal.muted,
                pal.background,
                w - pad * 2,
            )?;
        }
        for k in 0..count {
            let i = self.shown[k];
            let (col, row) = ((k % cols) as i32, (k / cols) as i32);
            let r = rect(
                (pad + col * tw) as i16,
                (y + row * th) as i16,
                tw - 4,
                th - 4,
            );
            let hit = Hit::App(i);
            let focused = self.targets.len() == self.focus;
            let hovered = self.hover == Some(hit);
            let bg = if focused {
                pal.selected
            } else if hovered {
                pal.surface
            } else {
                pal.background
            };
            if focused {
                self.fill(r, pal.accent)?;
                self.fill(
                    rect(
                        r.x + 1,
                        r.y + 1,
                        i32::from(r.width) - 2,
                        i32::from(r.height) - 2,
                    ),
                    bg,
                )?;
            } else {
                self.fill(r, bg)?;
            }
            let key = self.items[i].key.clone();
            let ix = r.x + ((i32::from(r.width) - isz) / 2) as i16;
            let iy = r.y + 6;
            let state = self.picture(&key, isz as u16);
            if state == Some(true) {
                if let Some(Some(p)) = self.pictures.get(&key) {
                    self.r
                        .image(&self.c, self.gc, ix, iy, p.size, &p.pixels, bg)?;
                }
            } else if state.is_none() {
                // Still being drawn: a quiet square in its place.
                self.fill(
                    rect(ix + isz as i16 / 4, iy + isz as i16 / 4, isz / 2, isz / 2),
                    pal.surface,
                )?;
            } else {
                // No icon: a letter tile in the theme's colours.
                let tile = rect(ix, iy, isz, isz);
                self.fill(tile, pal.border)?;
                self.fill(rect(ix + 2, iy + 2, isz - 4, isz - 4), pal.surface)?;
                let letter: String = self.items[i]
                    .name
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().collect())
                    .unwrap_or_else(|| "?".into());
                let b = self.baseline(iy, isz);
                self.text(
                    ix + ((isz - adv) / 2) as i16,
                    b,
                    &letter,
                    pal.warning,
                    pal.surface,
                    adv * 2,
                )?;
            }
            let name = self.items[i].name.clone();
            let max = (i32::from(r.width) - 6) / adv.max(1);
            let shown = truncate(&name, max as i16);
            let nw = shown.chars().count() as i32 * adv;
            let b = self.baseline(iy + isz as i16 + 4, lh);
            let colour = if focused { pal.foreground } else { pal.muted };
            self.text(
                r.x + ((i32::from(r.width) - nw) / 2).max(2) as i16,
                b,
                &shown,
                colour,
                bg,
                i32::from(r.width),
            )?;
            self.hits.push((r, hit));
            self.targets.push(hit);
        }
        Ok(y + th * self.app_rows as i32 + pad)
    }

    fn draw_cards(&mut self, y: i32, pad: i32) -> Result<i32> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let w = i32::from(self.w);
        let cards: Vec<String> = self
            .modules
            .iter()
            .filter(|m| matches!(m.as_str(), "today" | "system" | "updates" | "media"))
            .cloned()
            .collect();
        let cols = Self::card_columns(self.w, self.r.advance, cards.len());
        let gap = pad / 2;
        let cw = (w - pad * 2 - gap * (cols as i32 - 1)) / cols as i32;
        let ch = Self::card_height(lh);
        for (k, m) in cards.iter().enumerate() {
            let (col, row) = ((k % cols) as i32, (k / cols) as i32);
            let r = rect(
                (pad + col * (cw + gap)) as i16,
                (y + row * (ch + gap)) as i16,
                cw,
                ch,
            );
            let inner = cw - 16;
            match m.as_str() {
                "today" => {
                    let (x, ty) = self.card(r, "TODAY")?;
                    let lines = self.today_lines();
                    self.lines(x, ty, inner, &lines)?;
                    self.hits.push((r, Hit::Today));
                }
                "system" => {
                    let (x, ty) = self.card(r, "SYSTEM")?;
                    self.draw_system(x, ty, inner)?;
                    self.hits.push((r, Hit::System));
                }
                "updates" => {
                    let (x, ty) = self.card(r, "UPDATES")?;
                    let (lines, newer) = self.update_lines();
                    self.lines(x, ty, inner, &lines)?;
                    if newer {
                        let br = rect(
                            x,
                            r.y + ch as i16 - lh as i16 * 2 - 6,
                            inner.min(lh * 9),
                            lh + 10,
                        );
                        self.button(br, "Update…", Hit::Update, None)?;
                    }
                }
                _ => {
                    let (x, ty) = self.card(r, "NOW PLAYING")?;
                    self.draw_media(x, ty, inner, r)?;
                }
            }
            let _ = pal;
        }
        let rows = cards.len().div_ceil(cols) as i32;
        Ok(y + rows * (ch + gap) + gap)
    }

    fn today_lines(&self) -> Vec<(String, u32)> {
        let pal = self.cfg.palette;
        let mut out = Vec::new();
        let Some(v) = self.today.as_ref().and_then(|t| t.json.clone()) else {
            let (_, _, date, week) = clock();
            out.push((format!("{date} · week {week}"), pal.foreground));
            out.push((
                if self.today.is_some() {
                    "nirutoday: weather, events".into()
                } else {
                    "…".into()
                },
                pal.muted,
            ));
            return out;
        };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        out.push((
            format!(
                "{} · v. {}",
                s("date"),
                v.get("week").and_then(|x| x.as_u64()).unwrap_or(0)
            ),
            pal.foreground,
        ));
        if !s("temp").is_empty() {
            let mut t = format!("{} {} · {}", s("temp"), s("place"), s("sky"));
            if !s("high").is_empty() {
                t = format!("{t} · {}/{}", s("high"), s("low"));
            }
            out.push((t, pal.foreground));
        }
        let vma = v.get("vma").and_then(|x| x.as_u64()).unwrap_or(0);
        let warn = v.get("warnings").and_then(|x| x.as_u64()).unwrap_or(0);
        if vma > 0 {
            out.push((format!("VMA: {vma} important message(s)"), pal.error));
        } else if warn > 0 {
            out.push((format!("{warn} weather warning(s)"), pal.warning));
        }
        let events = v
            .get("events")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        for e in events.iter().take(2) {
            let t = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            out.push((format!("{} {}", t("time"), t("summary")), pal.success));
        }
        if events.is_empty() {
            let mut sun = String::new();
            if !s("sunrise").is_empty() {
                sun = format!("Sun {}–{}", s("sunrise"), s("sunset"));
            }
            if !s("names").is_empty() {
                out.push((format!("Name day: {}", s("names")), pal.muted));
            }
            if !sun.is_empty() {
                out.push((sun, pal.muted));
            }
        }
        out.truncate(5);
        out
    }

    fn draw_system(&mut self, x: i16, y: i16, w: i32) -> Result<()> {
        let pal = self.cfg.palette;
        let lh = self.r.line_height + 2;
        let mut row = 0i16;
        let mut line = |p: &mut Self, label: String, share: Option<f64>| -> Result<()> {
            let ty = y + row * lh;
            let b = p.baseline(ty, i32::from(lh));
            // The figure on the left, its meter on the right of the line.
            let mw = if share.is_some() { w * 3 / 10 } else { 0 };
            p.text(x, b, &label, pal.foreground, pal.surface, w - mw - 6)?;
            if let Some(s) = share {
                let colour = if s > 0.9 {
                    pal.error
                } else if s > 0.7 {
                    pal.warning
                } else {
                    pal.success
                };
                let mh = (i32::from(p.r.line_height) / 3).max(3);
                let my = ty + ((i32::from(lh) - mh) / 2) as i16;
                p.meter(x + (w - mw) as i16, my, mw, s, colour)?;
            }
            row += 1;
            Ok(())
        };
        let Some(sys) = self.system.clone() else {
            line(self, "…".into(), None)?;
            return Ok(());
        };
        line(
            self,
            format!("CPU {:>3.0} %", sys.cpu),
            Some(sys.cpu / 100.0),
        )?;
        if let Some((u, t)) = sys.memory {
            line(
                self,
                format!("RAM {}", data::human_bytes(u)),
                Some(u as f64 / t.max(1) as f64),
            )?;
        }
        if let Some((u, t)) = sys.disk {
            line(
                self,
                format!("Disk {} free", data::human_bytes(t.saturating_sub(u))),
                Some(u as f64 / t.max(1) as f64),
            )?;
        }
        let mut last = Vec::new();
        if let Some(c) = sys.temperature {
            last.push(format!("{c:.0} °C"));
        }
        if let Some((pct, charging)) = sys.battery {
            last.push(format!(
                "battery {pct} %{}",
                if charging { " ⚡" } else { "" }
            ));
        }
        if let Some(up) = sys.uptime {
            last.push(format!("up {}", data::human_uptime(up)));
        }
        if !last.is_empty() {
            line(self, last.join(" · "), None)?;
        }
        Ok(())
    }

    fn update_lines(&self) -> (Vec<(String, u32)>, bool) {
        let pal = self.cfg.palette;
        let Some(u) = &self.updates else {
            return (vec![("…".into(), pal.muted)], false);
        };
        let mut out = Vec::new();
        let installed = u.installed.clone().unwrap_or_else(|| "?".into());
        out.push((format!("NIRUX {installed}"), pal.foreground));
        match &u.newer {
            Some(v) => out.push((format!("NIRUX {v} is out"), pal.warning)),
            None => out.push(("NIRUX is up to date".into(), pal.success)),
        }
        match u.packages {
            Some(0) => out.push(("System packages up to date".into(), pal.success)),
            Some(n) => out.push((format!("{n} system package update(s)"), pal.warning)),
            None => {}
        }
        (out, u.newer.is_some())
    }

    fn draw_media(&mut self, x: i16, y: i16, w: i32, card: Rectangle) -> Result<()> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let bw = (w - 16) / 3;
        let by = card.y + card.height as i16 - lh as i16 * 2 - 6;
        match self.media.clone() {
            Some(m) => {
                let lines = vec![
                    (
                        if m.title.is_empty() {
                            "Playing".to_string()
                        } else {
                            m.title.clone()
                        },
                        pal.foreground,
                    ),
                    (m.artist.clone(), pal.muted),
                ];
                self.lines(x, y, w, &lines)?;
                self.button(rect(x, by, bw, lh + 10), "◀◀", Hit::Previous, None)?;
                let play = if m.playing { "❚❚" } else { "▶" };
                self.button(
                    rect(x + (bw + 8) as i16, by, bw, lh + 10),
                    play,
                    Hit::PlayPause,
                    None,
                )?;
                self.button(
                    rect(x + (bw + 8) as i16 * 2, by, bw, lh + 10),
                    "▶▶",
                    Hit::Next,
                    None,
                )?;
            }
            None => {
                let text = if self.media_known {
                    "Nothing plays"
                } else {
                    "…"
                };
                self.lines(x, y, w, &[(text.into(), pal.muted)])?;
                self.button(
                    rect(x, by, w.min(lh * 10), lh + 10),
                    "Open NIRUAMP",
                    Hit::OpenPlayer,
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn draw_toggles(&mut self, y: i32, pad: i32) -> Result<i32> {
        let lh = i32::from(self.r.line_height);
        let w = i32::from(self.w);
        let n = Toggle::ALL.len() as i32;
        let gap = pad / 2;
        let bw = (w - pad * 2 - gap * (n - 1)) / n;
        for (k, t) in Toggle::ALL.iter().enumerate() {
            let state = self.toggles.get(t).copied().flatten();
            let label = match state {
                Some(true) => format!("● {}", t.label()),
                Some(false) => format!("○ {}", t.label()),
                None if self.toggles.contains_key(t) => format!("– {}", t.label()),
                None => format!("… {}", t.label()),
            };
            let r = rect(
                (pad + k as i32 * (bw + gap)) as i16,
                y as i16,
                bw,
                lh * 2 - 4,
            );
            self.button(r, &label, Hit::Toggle(*t), state)?;
        }
        Ok(y + lh * 2 + pad / 2)
    }

    fn draw_power(&mut self, y: i32, pad: i32) -> Result<i32> {
        let lh = i32::from(self.r.line_height);
        let w = i32::from(self.w);
        let n = Power::ALL.len() as i32;
        let gap = pad / 2;
        let bw = (w - pad * 2 - gap * (n - 1)) / n;
        for (k, p) in Power::ALL.iter().enumerate() {
            let r = rect(
                (pad + k as i32 * (bw + gap)) as i16,
                y as i16,
                bw,
                lh * 2 - 4,
            );
            let armed = self.confirm == Some(*p);
            let label = if armed {
                format!("{}?", p.label())
            } else {
                p.label().to_string()
            };
            self.button(r, &label, Hit::Power(*p), armed.then_some(true))?;
        }
        Ok(y + lh * 2 + pad / 2)
    }

    // --------------------------------------------------------------- input

    /// Do what a target is for. `false` closes the panel.
    fn activate(&mut self, hit: Hit) -> bool {
        self.note.clear();
        if !matches!(hit, Hit::Power(_)) {
            self.confirm = None;
        }
        match hit {
            Hit::App(i) => {
                let item = self.items[i].clone();
                if let Target::Spawn {
                    argv,
                    terminal,
                    path,
                } = &item.target
                {
                    match spawn_command(argv, path.as_deref(), *terminal, &self.cfg) {
                        Ok(()) => {
                            record_usage(&item.key);
                            return false;
                        }
                        Err(e) => self.note = e,
                    }
                }
                true
            }
            Hit::PlayPause | Hit::Previous | Hit::Next => {
                let method = match hit {
                    Hit::PlayPause => "playback.toggle",
                    Hit::Previous => "playback.previous",
                    _ => "playback.next",
                };
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let _ = data::nplay(method);
                    std::thread::sleep(Duration::from_millis(250));
                    let _ = tx.send(Msg::Media(data::media()));
                });
                true
            }
            Hit::OpenPlayer => self.spawn(&["niruamp"]),
            Hit::Today => self.spawn(&["nirutoday"]),
            Hit::System => self.spawn(&["nirulauncher", "--health"]),
            Hit::Update => {
                let argv = vec!["nirux".to_string(), "update".to_string()];
                match spawn_terminal_command(&argv, true, &self.cfg) {
                    Ok(()) => false,
                    Err(e) => {
                        self.note = e;
                        true
                    }
                }
            }
            Hit::Toggle(t) => {
                let Some(now) = self.toggles.get(&t).copied().flatten() else {
                    self.note = format!("{} cannot be switched here", t.label());
                    return true;
                };
                let argv = t.command(!now);
                // Shown at once; the real state follows from the tool.
                self.toggles.insert(t, Some(!now));
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                    let _ = data::bounded(&refs, Duration::from_secs(5));
                    let _ = tx.send(Msg::Toggle(t, t.state()));
                });
                true
            }
            Hit::Power(p) => {
                if p.asks() && self.confirm != Some(p) {
                    self.confirm = Some(p);
                    return true;
                }
                self.close();
                let _ = std::process::Command::new("niructl")
                    .arg(p.command())
                    .stdin(std::process::Stdio::null())
                    .spawn();
                false
            }
        }
    }

    fn spawn(&mut self, argv: &[&str]) -> bool {
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        match spawn_command(&argv, None, false, &self.cfg) {
            Ok(()) => false,
            Err(e) => {
                self.note = e;
                true
            }
        }
    }

    /// Up and down move a row in the grid of apps, otherwise to the
    /// group before or after; left and right go one step.
    fn step(&mut self, sym: u32) {
        let n = self.targets.len();
        if n == 0 {
            return;
        }
        let apps = self
            .targets
            .iter()
            .filter(|t| matches!(t, Hit::App(_)))
            .count();
        let lh = i32::from(self.r.line_height);
        let isz = Self::icon_size(lh);
        let tw = (isz + lh * 2).max(i32::from(self.r.advance) * 11);
        let cols = ((i32::from(self.w) - lh * 2) / tw).max(1) as usize;
        let f = self.focus;
        self.focus = match sym {
            0xff51 => f.saturating_sub(1),
            0xff53 => (f + 1).min(n - 1),
            0xff52 if f < apps && f >= cols => f - cols,
            0xff54 if f + cols < apps => f + cols,
            0xff54 if f < apps => apps.min(n - 1),
            0xff52 if f >= apps && apps > 0 => {
                // Back up to the group before this one.
                let group = |h: &Hit| std::mem::discriminant(h);
                let mine = group(&self.targets[f]);
                let start = (0..f)
                    .rev()
                    .find(|&k| group(&self.targets[k]) != mine)
                    .unwrap_or(0);
                let prev = group(&self.targets[start]);
                (0..=start)
                    .rev()
                    .take_while(|&k| group(&self.targets[k]) == prev)
                    .last()
                    .unwrap_or(0)
            }
            0xff54 => {
                let group = |h: &Hit| std::mem::discriminant(h);
                let mine = group(&self.targets[f]);
                (f..n)
                    .find(|&k| group(&self.targets[k]) != mine)
                    .unwrap_or(f)
            }
            _ => f,
        };
    }

    fn key(&mut self, e: KeyPressEvent) -> Result<bool> {
        let sym = self.keymap.symbol(e.detail, e.state);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        self.note.clear();
        match sym {
            0xff1b => {
                if self.confirm.take().is_some() {
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.refilter();
                } else {
                    return Ok(false);
                }
            }
            0xff0d | 0xff8d => {
                if let Some(hit) = self.targets.get(self.focus).copied() {
                    if !self.activate(hit) {
                        return Ok(false);
                    }
                } else if !self.query.is_empty() {
                    // Nothing among the apps: the launcher searches further.
                    let q = self.query.clone();
                    self.close();
                    let _ = spawn_command(
                        &["nirulauncher".into(), "--apps".into(), "--query".into(), q],
                        None,
                        false,
                        &self.cfg,
                    );
                    return Ok(false);
                }
            }
            0xff09 => self.focus = (self.focus + 1) % self.targets.len().max(1),
            0xfe20 => {
                let n = self.targets.len().max(1);
                self.focus = (self.focus + n - 1) % n;
            }
            0xff51..=0xff54 => self.step(sym),
            0xff08 => {
                if ctrl {
                    delete_word(&mut self.query);
                } else {
                    self.query.pop();
                }
                self.refilter();
            }
            _ if ctrl && sym == 0x75 => {
                self.query.clear();
                self.refilter();
            }
            _ => {
                if let Some(ch) = key_char(sym).filter(|_| !ctrl) {
                    self.query.push(ch);
                    self.refilter();
                }
            }
        }
        Ok(true)
    }

    fn run(&mut self) -> Result<()> {
        use std::os::fd::AsRawFd;
        let fd = self.c.stream().as_raw_fd();
        loop {
            let mut dirty = false;
            while let Ok(msg) = self.rx.try_recv() {
                dirty = true;
                match msg {
                    Msg::Today(t) => self.today = Some(t),
                    Msg::Updates(u) => self.updates = Some(u),
                    Msg::Media(m) => {
                        self.media = m;
                        self.media_known = true;
                    }
                    Msg::Toggle(t, s) => {
                        self.toggles.insert(t, s);
                    }
                    Msg::System(sys) => {
                        self.system = Some(sys);
                        self.system_busy = false;
                    }
                    Msg::Icon(key, p) => {
                        self.pending.remove(&key);
                        self.pictures.insert(key, p);
                    }
                }
            }
            // The system figures and the clock every two seconds, what plays
            // every three, only while the panel is open.
            if self.last_tick.elapsed() >= Duration::from_secs(2) {
                self.last_tick = Instant::now();
                self.poll_system(Duration::from_millis(1000));
                dirty = true;
            }
            if self.has("media") && self.last_media.elapsed() >= Duration::from_secs(3) {
                self.last_media = Instant::now();
                self.poll_media();
            }
            while let Some(event) = self.c.poll_for_event()? {
                match event {
                    Event::Expose(_) => dirty = true,
                    Event::KeyPress(e) => {
                        if !self.key(e)? {
                            return Ok(());
                        }
                        dirty = true;
                    }
                    Event::ButtonPress(e) => {
                        if e.event != self.win
                            && !(e.event == self.root
                                && inside(
                                    rect(0, 0, i32::from(self.w), i32::from(self.h)),
                                    e.event_x,
                                    e.event_y,
                                ))
                        {
                            // A click outside closes, as the launcher does.
                            return Ok(());
                        }
                        if e.detail != 1 {
                            continue;
                        }
                        let hit = self
                            .hits
                            .iter()
                            .rev()
                            .find(|(r, _)| inside(*r, e.event_x, e.event_y))
                            .map(|(_, h)| *h);
                        if let Some(hit) = hit {
                            if let Some(k) = self.targets.iter().position(|t| *t == hit) {
                                self.focus = k;
                            }
                            if !self.activate(hit) {
                                return Ok(());
                            }
                        }
                        dirty = true;
                    }
                    Event::MotionNotify(e) if e.event == self.win => {
                        let hit = self
                            .hits
                            .iter()
                            .rev()
                            .find(|(r, _)| inside(*r, e.event_x, e.event_y))
                            .map(|(_, h)| *h);
                        if hit != self.hover {
                            self.hover = hit;
                            dirty = true;
                        }
                    }
                    Event::ClientMessage(e) if e.type_ == self.activate => return Ok(()),
                    Event::SelectionClear(_) => return Ok(()),
                    _ => {}
                }
            }
            if dirty {
                self.draw()?;
            }
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut pfd, 1, 250) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_weeks_match_the_calendar() {
        // 2026-11-25 is a Wednesday, day 328 of the year (0-based).
        assert_eq!(iso_week(2026, 328, 3), 48);
        // 2027-01-01 is a Friday: still week 53 of 2026.
        assert_eq!(iso_week(2027, 0, 5), 53);
        // 2026-01-01 is a Thursday: week 1.
        assert_eq!(iso_week(2026, 0, 4), 1);
        // 2025-12-29 is a Monday: week 1 of 2026.
        assert_eq!(iso_week(2025, 362, 1), 1);
    }

    #[test]
    fn the_modules_come_from_the_config() {
        let cfg = |t: &str| Config::from_value(&t.parse::<toml::Value>().unwrap()).unwrap();
        assert_eq!(cfg("").panel.modules, MODULES);
        let c = cfg("[panel]\nmodules = [\"power\", \"apps\", \"nonsense\", \"apps\"]\napp_rows = 9");
        assert_eq!(c.panel.modules, ["power", "apps"]);
        assert_eq!(c.panel.app_rows, 4);
        assert_eq!(cfg("[panel]\nmodules = []").panel.modules, MODULES);
    }

    #[test]
    fn the_logo_letters_are_drawn() {
        for ch in "NIRUPANEL".chars() {
            let g = glyph(ch);
            assert!(g.iter().all(|row| row.len() == 5));
            assert!(g.iter().any(|row| row.contains('#')), "{ch}");
        }
    }

    #[test]
    fn the_panel_fits_its_modules() {
        let all: Vec<String> = MODULES.iter().map(|s| s.to_string()).collect();
        let tall = Panel::height_for(&all, 2, 19, 9, 1000);
        let short = Panel::height_for(&all, 1, 19, 9, 1000);
        assert!(tall > short);
        let none: Vec<String> = vec!["power".into()];
        assert!(Panel::height_for(&none, 2, 19, 9, 1000) < short);
        assert_eq!(Panel::card_columns(1000, 9, 4), 4);
        assert_eq!(Panel::card_columns(720, 9, 4), 3);
        assert_eq!(Panel::card_columns(400, 9, 4), 1);
    }
}
