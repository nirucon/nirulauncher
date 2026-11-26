//! NIRUPANEL: a floating panel in the middle of the screen (`Super+Space`,
//! `nirulauncher --panel`). A retro logo, a search field over the apps with
//! their icons, and modules: today, the system, updates, what plays, the
//! clipboard, notifications, quick switches, power and the computers in the
//! tailnet. Typing searches everything at once (`search.rs`): the apps, and
//! in groups below them settings and actions, key bindings, computers, the
//! manual and files. Which modules show, and in which order, is
//! `[panel] modules` in `~/.config/nirulauncher/config.toml`; what the
//! search finds is `[panel] search`.
//!
//! It is the launcher's own process and its own parts (desktop entries,
//! ranking, usage, spawning), not a second launcher. Every source is local
//! and the slow ones run on worker threads, so the first frame is drawn at
//! once and the modules fill in.
mod data;
mod icons;
mod search;
mod wrap;

pub(crate) use search::SEARCH;

use crate::config::Config;
use crate::render::Renderer;
use crate::tailscale::Device;
use crate::*;
use data::Toggle;
use search::{Group, Source};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use wrap::{fit, together, wrap};
use x11rb::protocol::xproto::*;

/// The modules there are, in their default order. `keys` is not drawn on
/// its own: it lets the search find key bindings.
pub(crate) const MODULES: [&str; 11] = [
    "apps",
    "today",
    "system",
    "updates",
    "media",
    "clipboard",
    "notifications",
    "computers",
    "toggles",
    "power",
    "keys",
];

/// The modules drawn as cards, in a grid.
const CARDS: [&str; 6] = [
    "today",
    "system",
    "updates",
    "media",
    "clipboard",
    "notifications",
];

/// The modules below the apps that search results stand in for.
const BODY: [&str; 7] = [
    "today",
    "system",
    "updates",
    "media",
    "clipboard",
    "notifications",
    "computers",
];

/// Clipboard entries the clipboard card reads.
const CLIPS: usize = 8;

/// The room search results get when there is no card or computer, in lines.
const RESULT_LINES: i32 = 12;

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
    /// A computer of `computers`, by its place in the list shown.
    Computer(usize),
    /// A key binding, by its place in `keys`.
    Key(usize),
    /// A setting or action the search found, by its place in `settings`.
    Setting(usize),
    /// A manual topic, by its place in `manual`.
    Manual(usize),
    /// A file the search found, by its place in the file rows.
    File(usize),
    /// A clipboard entry on the clipboard card.
    Clip(usize),
    /// The clipboard card: the launcher's clipboard view.
    Clipboard,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FileAction {
    Open,
    /// The folder it is in, in the file manager.
    Folder,
    CopyPath,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ComputerAction {
    /// `ssh NAME` in the NIRUX terminal.
    Ssh,
    /// `sftp://NAME/` in the file manager (`xdg-open`).
    Files,
    CopyIp,
    CopyName,
}

/// Groups of targets that Up and Down move between.
fn group_of(hit: Hit) -> u8 {
    match hit {
        Hit::App(_) => 0,
        Hit::Previous | Hit::PlayPause | Hit::Next | Hit::OpenPlayer | Hit::Update => 1,
        Hit::Today | Hit::System => 2,
        Hit::Computer(_) => 3,
        Hit::Key(_) => 4,
        Hit::Toggle(_) => 5,
        Hit::Power(_) => 6,
        Hit::Setting(_) => 7,
        Hit::Manual(_) => 8,
        Hit::File(_) => 9,
        Hit::Clip(_) | Hit::Clipboard => 10,
    }
}

/// Targets in the search results (two columns: left and right move across).
fn is_result(hit: Hit) -> bool {
    matches!(
        hit,
        Hit::Setting(_) | Hit::Key(_) | Hit::Manual(_) | Hit::File(_)
    )
}

/// What the workers send back.
enum Msg {
    Today(data::Today),
    Updates(data::Updates),
    Media(Option<data::Media>),
    Toggle(Toggle, Option<bool>),
    Icon(String, Option<icons::Picture>),
    System(data::System),
    Computers(std::result::Result<Vec<Device>, String>),
    /// The rows of a search group, read once.
    Rows(Group, Vec<Item>),
    Clips(Vec<Item>),
    Notices(std::result::Result<data::Notices, String>),
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
    /// The computers shown, once Tailscale has answered.
    computers: Option<std::result::Result<Vec<Device>, String>>,
    computer_rows: usize,
    /// Computers in a row, as last drawn (for Up and Down).
    computer_cols: usize,
    /// What the search finds besides apps (`[panel] search`).
    search: Vec<String>,
    /// NIRUWM's key bindings, read once on the first search.
    keys: Source,
    /// The launcher's settings and actions, read once on the first search.
    settings: Source,
    /// The manual's chapters and headings, read once on the first search.
    manual: Source,
    /// The computers as rows to search (row `k` is computer `k`).
    computer_rows_found: Source,
    /// The file search (the Files view's), on a worker a newer query cancels.
    files: crate::modes::files::State,
    /// When the search last changed, for `NIRUPANEL_DEBUG`.
    searched: Instant,
    /// The clipboard card's entries, newest first.
    clips: Option<Vec<Item>>,
    /// The notifications card.
    notices: Option<std::result::Result<data::Notices, String>>,
    /// A key binding that ends the session or closes a window waits for a
    /// second Enter.
    confirm_key: Option<usize>,
    /// The focus was moved by hand since the search last changed.
    moved: bool,
    /// `NIRUPANEL_DEBUG=hits`: every frame's targets on stderr, for tests.
    debug_hits: bool,
    /// `NIRUPANEL_DEBUG` set: timings on stderr.
    debug: bool,
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

/// The keys of a key binding row ("Super+T"), without its conflict mark.
fn key_label(item: &Item) -> String {
    item.detail.split(" · ").next().unwrap_or("").to_string()
}

/// A key binding that ends the session, sleeps or closes a window: Enter
/// asks once more before it runs.
fn risky_key(item: &Item) -> bool {
    let what = crate::compose::fold(
        &format!(
            "{} {}",
            item.name,
            item.keywords
                .split(crate::modes::keys::NOTE)
                .next()
                .unwrap_or("")
        )
        .to_lowercase(),
    );
    let words: Vec<&str> = what
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    const RISKY: [&str; 10] = [
        "quit",
        "logout",
        "reboot",
        "restart",
        "shutdown",
        "poweroff",
        "suspend",
        "hibernate",
        "close",
        "kill",
    ];
    words.iter().any(|w| RISKY.contains(w))
        || what.contains("log out")
        || what.contains("shut down")
        || what.contains("power off")
}

/// What a computer is: (text, whether it is this computer).
fn computer_detail(d: &Device, now: i64) -> (String, bool) {
    let os = match d.os.as_str() {
        "" => "?".to_string(),
        "macOS" | "iOS" | "tvOS" => d.os.clone(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    };
    if d.this {
        return (format!("this computer · {os}"), true);
    }
    let mut parts = vec![os];
    if d.online {
        parts.push("online".into());
        match d.direct {
            Some(true) => parts.push("direct".into()),
            Some(false) if !d.relay.is_empty() => parts.push(format!("relay {}", d.relay)),
            _ => {}
        }
        if d.exit_node {
            parts.push("exit node".into());
        }
    } else {
        parts.push("offline".into());
        if let Some(t) = d.last_seen {
            parts.push(format!("seen {}", crate::tailscale::ago(now, t)));
        }
    }
    let parts: Vec<String> = parts.iter().map(|p| together(p)).collect();
    (parts.join(" · "), false)
}

/// A host name or address from Tailscale that is safe to pass to `ssh` and
/// put in a URI: letters, digits, `-`, `.`, `:` and `_`, not starting
/// with `-` (an option).
fn safe_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_'))
}

/// Where an arrow key moves the focus among targets in `groups` (one group
/// id per target, in order). `cols(group)` is the width of a grid group
/// (0: not a grid, Up and Down leave it).
fn step_in(groups: &[u8], f: usize, sym: u32, cols: impl Fn(u8) -> usize) -> usize {
    let n = groups.len();
    if n == 0 {
        return 0;
    }
    let g = groups[f];
    let start = (0..f)
        .rev()
        .take_while(|&k| groups[k] == g)
        .last()
        .unwrap_or(f);
    let end = (f..n).find(|&k| groups[k] != g).unwrap_or(n);
    let c = cols(g);
    match sym {
        0xff51 => f.saturating_sub(1),
        0xff53 => (f + 1).min(n - 1),
        0xff52 if c > 0 && f - start >= c => f - c,
        0xff52 if start > 0 => {
            // The start of the group before.
            let p = groups[start - 1];
            (0..start)
                .rev()
                .take_while(|&k| groups[k] == p)
                .last()
                .unwrap_or(0)
        }
        0xff54 if c > 0 && f + c < end => f + c,
        0xff54 if end < n => end,
        _ => f,
    }
}

/// Whether the search finds `group` with these modules and `[panel]
/// search`: apps, key bindings and computers need their modules too.
fn finds(modules: &[String], search: &[String], group: &str) -> bool {
    let on = |list: &[String], m: &str| list.iter().any(|x| x == m);
    on(search, group) && (!matches!(group, "apps" | "keys" | "computers") || on(modules, group))
}

/// Whether the search finds anything besides apps (results below them).
fn has_results(modules: &[String], search: &[String]) -> bool {
    SEARCH
        .iter()
        .any(|g| *g != "apps" && finds(modules, search, g))
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
        let mut modules = cfg.panel.modules.clone();
        // No Tailscale, no computers (and nothing is run to find out).
        if modules.iter().any(|m| m == "computers") && !data::on_path("tailscale") {
            modules.retain(|m| m != "computers");
        }
        let mut app_rows = cfg.panel.app_rows;
        let mut computer_rows = cfg.panel.computer_rows;
        // Tall enough for every module; on a small screen fewer rows of
        // computers, then of apps.
        let results = has_results(&modules, &cfg.panel.search);
        let mut h = Self::height_for(&modules, app_rows, computer_rows, line, advance, w, results);
        while h > i32::from(monitor.h) - i32::from(line) * 2 && (app_rows > 1 || computer_rows > 1)
        {
            if computer_rows > 1 {
                computer_rows -= 1;
            } else {
                app_rows -= 1;
            }
            h = Self::height_for(&modules, app_rows, computer_rows, line, advance, w, results);
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
        let cfg_search = cfg.panel.search.clone();
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
            computers: None,
            computer_rows,
            computer_cols: 1,
            search: cfg_search,
            keys: Source::default(),
            settings: Source::default(),
            manual: Source::default(),
            computer_rows_found: Source::default(),
            files: Default::default(),
            searched: Instant::now(),
            clips: None,
            notices: None,
            confirm_key: None,
            moved: false,
            debug_hits: std::env::var_os("NIRUPANEL_DEBUG").is_some_and(|v| v == "hits"),
            debug: std::env::var_os("NIRUPANEL_DEBUG").is_some(),
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

    /// Cards in a row for the panel's width: as many as fit, and the rows
    /// as even as they can be (six cards where four fit: three and three).
    fn card_columns(w: u16, advance: i16, cards: usize) -> usize {
        let fit = (i32::from(w) / (i32::from(advance) * 26)).max(1) as usize;
        let cards = cards.max(1);
        let rows = cards.div_ceil(fit);
        cards.div_ceil(rows).max(1)
    }

    fn height_for(
        modules: &[String],
        app_rows: usize,
        computer_rows: usize,
        line: i16,
        advance: i16,
        w: u16,
        results: bool,
    ) -> i32 {
        let lh = i32::from(line);
        let pad = lh;
        let mut h = pad + Self::logo_height(lh) + pad / 2 + lh * 2 + pad;
        let has = |m: &str| modules.iter().any(|x| x == m);
        if has("apps") {
            h += Self::tile_height(lh) * app_rows as i32 + pad;
        }
        h += Self::body_height(modules, computer_rows, line, advance, w, results);
        for m in ["toggles", "power"] {
            if has(m) {
                h += lh * 2 + pad / 2;
            }
        }
        h + lh + pad
    }

    /// The height of the cards and the computers (or, while searching, of
    /// the results in their place). Without them, search results get room
    /// of their own.
    fn body_height(
        modules: &[String],
        computer_rows: usize,
        line: i16,
        advance: i16,
        w: u16,
        results: bool,
    ) -> i32 {
        let lh = i32::from(line);
        let gap = lh / 2;
        let has = |m: &str| modules.iter().any(|x| x == m);
        let mut h = 0;
        let cards = CARDS.iter().filter(|m| has(m)).count();
        if cards > 0 {
            let cols = Self::card_columns(w, advance, cards);
            let rows = cards.div_ceil(cols) as i32;
            h += rows * (Self::card_height(lh) + gap) + gap;
        }
        if has("computers") {
            h += Self::computers_height(lh, computer_rows) + gap;
        }
        if h == 0 && results {
            h = Self::results_height(lh);
        }
        h
    }

    /// The room search results get when there is no card or computer.
    fn results_height(lh: i32) -> i32 {
        lh * RESULT_LINES
    }

    /// One computer: its name and two lines about it.
    fn cell_height(lh: i32) -> i32 {
        (lh + 2) * 3 + 6
    }

    fn computers_height(lh: i32, rows: usize) -> i32 {
        lh + 10 + rows as i32 * (Self::cell_height(lh) + 4) + 4
    }

    fn logo_height(lh: i32) -> i32 {
        (lh / 5).max(2) * 7 + 6
    }
    /// An app: its icon and its name on up to two lines.
    fn tile_height(lh: i32) -> i32 {
        Self::icon_size(lh) + lh * 2 + 16
    }
    fn icon_size(lh: i32) -> i32 {
        (lh * 2 + lh / 2).clamp(32, 64)
    }
    /// A card: its title and six or seven lines of text.
    fn card_height(lh: i32) -> i32 {
        lh * 9 + lh / 2
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
        if self.has("computers") {
            let tx = self.tx.clone();
            let (hosts, hide) = (
                self.cfg.panel.computer_hosts.clone(),
                self.cfg.panel.computer_hide.clone(),
            );
            std::thread::spawn(move || {
                let list = data::computers().map(|all| crate::tailscale::pick(&all, &hosts, &hide));
                let _ = tx.send(Msg::Computers(list));
            });
        }
        if self.has("clipboard") {
            // NIRUCLIPBOARD's history file, as the clipboard view reads it.
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let mut clips = crate::modes::actions::clipboard_items();
                clips.truncate(CLIPS);
                let _ = tx.send(Msg::Clips(clips));
            });
        }
        if self.has("notifications") {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Notices(data::notifications()));
            });
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

    fn finds(&self, group: &str) -> bool {
        finds(&self.modules, &self.search, group)
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_string();
        let t = Instant::now();
        // Apps left out of `[panel] search` step aside while searching.
        self.shown = if self.finds("apps") || q.is_empty() {
            ranked_for_mode(&self.items, &q, &self.usage, "apps").order
        } else {
            Vec::new()
        };
        self.focus = 0;
        self.confirm = None;
        self.confirm_key = None;
        self.moved = false;
        self.searched = Instant::now();
        self.filter_results();
        if self.debug && !q.is_empty() {
            eprintln!(
                "nirupanel: search '{q}' ranked in {} us",
                t.elapsed().as_micros()
            );
        }
    }

    /// What the search finds below the apps. The first search starts the
    /// workers that read the key bindings, the settings and the manual;
    /// every search starts a file search (two letters or more) that a newer
    /// one cancels. Nothing here waits.
    fn filter_results(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() {
            for s in [
                &mut self.keys,
                &mut self.settings,
                &mut self.manual,
                &mut self.computer_rows_found,
            ] {
                s.shown.clear();
            }
            self.files.reset();
            return;
        }
        for g in Group::NEAR {
            if !self.finds(g.name()) {
                continue;
            }
            let usage = &self.usage;
            let source = match g {
                Group::Keys => &mut self.keys,
                Group::Settings => &mut self.settings,
                Group::Manual => &mut self.manual,
                _ => &mut self.computer_rows_found,
            };
            source.filter(&q, usage, g);
            if g != Group::Computers && source.ask() {
                let tx = self.tx.clone();
                let cfg = self.cfg.clone();
                std::thread::spawn(move || {
                    let rows = match g {
                        Group::Keys => search::key_items(),
                        Group::Settings => search::settings_items(&cfg),
                        _ => search::manual_items(),
                    };
                    let _ = tx.send(Msg::Rows(g, rows));
                });
            }
        }
        if self.finds("files") && q.chars().count() >= 2 {
            self.files.request(&self.cfg.files, &q);
        } else {
            self.files.reset();
        }
    }

    fn source(&self, g: Group) -> &Source {
        match g {
            Group::Keys => &self.keys,
            Group::Settings => &self.settings,
            Group::Manual => &self.manual,
            _ => &self.computer_rows_found,
        }
    }

    /// Whether search results stand in for the cards and computers now.
    fn showing_results(&self) -> bool {
        !self.query.trim().is_empty() && has_results(&self.modules, &self.search)
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

    /// Text on a card from `y` down to `bottom`: each entry wrapped to the
    /// width on up to three lines, as many as the room holds (see
    /// `wrap::fit`).
    fn lines(
        &mut self,
        x: i16,
        y: i16,
        w: i32,
        bottom: i16,
        entries: &[(String, u32)],
    ) -> Result<()> {
        let lh = self.r.line_height + 2;
        let bg = self.cfg.palette.surface;
        let chars = (w / i32::from(self.r.advance).max(1)).max(1) as usize;
        let budget = ((i32::from(bottom) - i32::from(y)) / i32::from(lh)).max(0) as usize;
        for (i, (t, c)) in fit(entries, chars, budget, 3).iter().enumerate() {
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
            let hint = self.hint();
            self.text(field.x + 10, b, "›", pal.warning, pal.surface, adv * 2)?;
            self.text(
                field.x + 10 + adv as i16 * 2,
                b,
                &hint,
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
        let searching = self.showing_results();
        let has_body = modules.iter().any(|m| BODY.contains(&m.as_str()));
        let own_room = !has_body && has_results(&modules, &self.search);
        let body = Self::body_height(
            &modules,
            self.computer_rows,
            self.r.line_height,
            self.r.advance,
            self.w,
            own_room,
        );
        // Without cards or computers the results get room of their own,
        // below the apps (or at the top without them).
        let mut results_done = !own_room;
        let mut cards_done = false;
        let mut body_done = false;
        if own_room && !self.has("apps") {
            results_done = true;
            y = self.draw_results_room(y, body, pad, searching)?;
        }
        for m in &modules {
            match m.as_str() {
                "apps" => {
                    y = self.draw_apps(y, pad)?;
                    if !std::mem::replace(&mut results_done, true) {
                        y = self.draw_results_room(y, body, pad, searching)?;
                    }
                }
                // Search results take the place of the cards and the
                // computers, at the first of them, in the same height.
                b if searching && BODY.contains(&b) => {
                    if !std::mem::replace(&mut body_done, true) {
                        y = self.draw_results(y, body, pad)?;
                    }
                }
                c if CARDS.contains(&c) => {
                    if !std::mem::replace(&mut cards_done, true) {
                        y = self.draw_cards(y, pad)?;
                    }
                }
                "computers" => y = self.draw_computers(y, pad)?,
                "toggles" => y = self.draw_toggles(y, pad)?,
                "power" => y = self.draw_power(y, pad)?,
                _ => {}
            }
        }
        // The footer: a notice, or the keys.
        let focused = self.targets.get(self.focus).copied();
        let footer = if !self.note.is_empty() {
            (self.note.clone(), pal.warning)
        } else if let Some(k) = self.confirm_key {
            let what = self
                .keys
                .get(k)
                .map(|i| format!("{} ({})", i.name, key_label(i)))
                .unwrap_or_default();
            (
                format!("{what}? Enter or a click again runs it, Esc cancels"),
                pal.warning,
            )
        } else if let Some(p) = self.confirm {
            (
                format!(
                    "{}? Enter or a click again to confirm, Esc to cancel",
                    p.label()
                ),
                pal.warning,
            )
        } else if let Some(Hit::Computer(i)) = self
            .hover
            .or(focused)
            .filter(|h| matches!(h, Hit::Computer(_)))
        {
            let name = self.computer(i).map(|d| d.name.clone()).unwrap_or_default();
            (
                format!(
                    "{name}: Enter or click ssh · Shift+Enter or middle-click files · Ctrl+C or right-click copy IP · Ctrl+Shift+C copy name"
                ),
                pal.muted,
            )
        } else if let Some(Hit::Key(i)) = focused {
            let runs = self
                .keys
                .get(i)
                .is_some_and(|v| !matches!(v.target, Target::Info));
            (
                if runs {
                    "Enter runs the key binding · Super+/ lists every key · Esc clears".into()
                } else {
                    "Shown only: it cannot be run from here · Super+/ lists every key".into()
                },
                pal.muted,
            )
        } else if let Some(Hit::File(_)) =
            self.hover.or(focused).filter(|h| matches!(h, Hit::File(_)))
        {
            (
                "Enter or click opens · Shift+Enter or middle-click opens its folder · Ctrl+C or right-click copies the path".into(),
                pal.muted,
            )
        } else if let Some(Hit::Setting(_) | Hit::Manual(_)) = focused {
            (
                "Enter opens · arrows move between the groups · Esc clears".into(),
                pal.muted,
            )
        } else if let Some(Hit::Clip(_)) =
            self.hover.or(focused).filter(|h| matches!(h, Hit::Clip(_)))
        {
            (
                "Enter or a click copies it back to the clipboard · a click on the title opens the history".into(),
                pal.muted,
            )
        } else if self.showing_results() {
            (
                "Enter opens the first · arrows and Tab move · Esc clears · Enter with nothing found searches the launcher".into(),
                pal.muted,
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
        if self.debug_hits {
            let list: Vec<String> = self
                .hits
                .iter()
                .map(|(r, h)| format!("{h:?}@{},{},{},{}", r.x, r.y, r.width, r.height))
                .collect();
            eprintln!(
                "nirupanel: frame {}x{} focus {:?} hits {}",
                self.w,
                self.h,
                self.targets.get(self.focus),
                list.join(" ")
            );
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
            let text = if !self.finds("apps") {
                "Apps are not searched here ([panel] search); the rest is below."
            } else if self.showing_results() {
                "No app matches; what else does is below."
            } else {
                "No app matches. Enter searches the launcher."
            };
            self.text(pad as i16, b, text, pal.muted, pal.background, w - pad * 2)?;
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
            let max = ((i32::from(r.width) - 6) / adv.max(1)).max(1) as usize;
            let colour = if focused { pal.foreground } else { pal.muted };
            for (n, shown) in wrap(&name, max, 2).iter().enumerate() {
                let nw = shown.chars().count() as i32 * adv;
                let b = self.baseline(iy + isz as i16 + 4 + (n as i32 * lh) as i16, lh);
                self.text(
                    r.x + ((i32::from(r.width) - nw) / 2).max(2) as i16,
                    b,
                    shown,
                    colour,
                    bg,
                    i32::from(r.width),
                )?;
            }
            self.hits.push((r, hit));
            self.targets.push(hit);
        }
        Ok(y + th * self.app_rows as i32 + pad)
    }

    fn draw_cards(&mut self, y: i32, pad: i32) -> Result<i32> {
        let lh = i32::from(self.r.line_height);
        let w = i32::from(self.w);
        let cards: Vec<String> = self
            .modules
            .iter()
            .filter(|m| CARDS.contains(&m.as_str()))
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
            // The last line of text ends above the card's border, or above
            // its buttons.
            let bottom = r.y + ch as i16 - 4;
            let above_buttons = r.y + ch as i16 - lh as i16 * 2 - 8;
            match m.as_str() {
                "today" => {
                    let (x, ty) = self.card(r, "TODAY")?;
                    let lines = self.today_lines();
                    self.lines(x, ty, inner, bottom, &lines)?;
                    self.hits.push((r, Hit::Today));
                }
                "system" => {
                    let (x, ty) = self.card(r, "SYSTEM")?;
                    self.draw_system(x, ty, inner, bottom)?;
                    self.hits.push((r, Hit::System));
                }
                "updates" => {
                    let (x, ty) = self.card(r, "UPDATES")?;
                    let (lines, newer) = self.update_lines();
                    let end = if newer { above_buttons } else { bottom };
                    self.lines(x, ty, inner, end, &lines)?;
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
                "clipboard" => {
                    let (x, ty) = self.card(r, "CLIPBOARD")?;
                    self.hits.push((r, Hit::Clipboard));
                    self.draw_clips(x, ty, inner, bottom)?;
                }
                "notifications" => {
                    let title = match &self.notices {
                        Some(Ok(n)) if n.dnd => "NOTIFICATIONS · do not disturb",
                        _ => "NOTIFICATIONS",
                    };
                    let (x, ty) = self.card(r, title)?;
                    let lines = self.notice_lines();
                    self.lines(x, ty, inner, bottom, &lines)?;
                }
                _ => {
                    let (x, ty) = self.card(r, "NOW PLAYING")?;
                    self.draw_media(x, ty, inner, r, above_buttons)?;
                }
            }
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
        out
    }

    fn draw_system(&mut self, x: i16, y: i16, w: i32, bottom: i16) -> Result<()> {
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
        // Each figure stays on one line with its unit.
        if let Some(c) = sys.temperature {
            last.push(together(&format!("{c:.0} °C")));
        }
        if let Some((pct, charging)) = sys.battery {
            last.push(together(&format!(
                "battery {pct} %{}",
                if charging { " ⚡" } else { "" }
            )));
        }
        if let Some(up) = sys.uptime {
            last.push(together(&format!("up {}", data::human_uptime(up))));
        }
        if !last.is_empty() {
            // The rest on as many lines as it needs and the card holds.
            let ty = y + row * lh;
            self.lines(x, ty, w, bottom, &[(last.join(" · "), pal.foreground)])?;
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

    fn draw_media(&mut self, x: i16, y: i16, w: i32, card: Rectangle, bottom: i16) -> Result<()> {
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
                self.lines(x, y, w, bottom, &lines)?;
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
                self.lines(x, y, w, bottom, &[(text.into(), pal.muted)])?;
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

    fn computer(&self, i: usize) -> Option<&Device> {
        self.computers.as_ref()?.as_ref().ok()?.get(i)
    }

    /// The computers in a card: a grid of cells with the name, online in
    /// the success colour or offline muted, and what each one is.
    fn draw_computers(&mut self, y: i32, pad: i32) -> Result<i32> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance).max(1);
        let w = i32::from(self.w);
        let gap = pad / 2;
        let height = Self::computers_height(lh, self.computer_rows);
        let r = rect(pad as i16, y as i16, w - pad * 2, height);
        let (x, ty) = self.card(r, "COMPUTERS")?;
        let inner = i32::from(r.width) - 16;
        let bottom = r.y + r.height as i16 - 4;
        let list = match self.computers.clone() {
            None => {
                self.lines(x, ty, inner, bottom, &[("…".into(), pal.muted)])?;
                return Ok(y + height + gap);
            }
            Some(Err(e)) => {
                self.lines(x, ty, inner, bottom, &[(e, pal.muted)])?;
                return Ok(y + height + gap);
            }
            Some(Ok(list)) if list.is_empty() => {
                let text = "No computers to show: see [panel.computers] hosts and hide";
                self.lines(x, ty, inner, bottom, &[(text.into(), pal.muted)])?;
                return Ok(y + height + gap);
            }
            Some(Ok(list)) => list,
        };
        let cell_gap = 6;
        let cols = ((inner + cell_gap) / (adv * 24 + cell_gap)).max(1) as usize;
        let cw = (inner - cell_gap * (cols as i32 - 1)) / cols as i32;
        let chh = Self::cell_height(lh);
        self.computer_cols = cols;
        let room = cols * self.computer_rows;
        // More than there is room for: the last cell says how many more.
        let shown = if list.len() > room {
            room.saturating_sub(1)
        } else {
            list.len()
        };
        let now = data::now();
        let row_h = (lh + 2) as i16;
        let chars = ((cw - 12) / adv).max(1) as usize;
        for (k, d) in list.iter().enumerate().take(shown) {
            let (col, row) = ((k % cols) as i32, (k / cols) as i32);
            let cell = rect(
                x + (col * (cw + cell_gap)) as i16,
                ty + (row * (chh + 4)) as i16,
                cw,
                chh,
            );
            let hit = Hit::Computer(k);
            let focused = self.targets.len() == self.focus;
            let bg = if focused || self.hover == Some(hit) {
                pal.selected
            } else {
                pal.background
            };
            self.fill(cell, if focused { pal.accent } else { bg })?;
            self.fill(
                rect(
                    cell.x + 1,
                    cell.y + 1,
                    i32::from(cell.width) - 2,
                    i32::from(cell.height) - 2,
                ),
                bg,
            )?;
            let (dot, name_colour) = if d.online {
                (pal.success, pal.foreground)
            } else {
                (pal.muted, pal.muted)
            };
            let tx = cell.x + 6;
            let b = self.baseline(cell.y + 3, i32::from(row_h));
            self.text(tx, b, "●", dot, bg, adv * 2)?;
            let name = wrap(&d.name, chars.saturating_sub(2).max(1), 1).remove(0);
            self.text(
                tx + adv as i16 * 2,
                b,
                &name,
                name_colour,
                bg,
                (chars as i32 - 2) * adv,
            )?;
            let (detail, colour) = computer_detail(d, now);
            let colour = if colour { pal.warning } else { pal.muted };
            for (n, l) in wrap(&detail, chars, 2).iter().enumerate() {
                let b = self.baseline(cell.y + 3 + row_h * (n as i16 + 1), i32::from(row_h));
                self.text(tx, b, l, colour, bg, chars as i32 * adv)?;
            }
            self.hits.push((cell, hit));
            self.targets.push(hit);
        }
        if shown < list.len() {
            let k = shown;
            let (col, row) = ((k % cols) as i32, (k / cols) as i32);
            let cx = x + (col * (cw + cell_gap)) as i16 + 6;
            let cy = ty + (row * (chh + 4)) as i16 + 3;
            let more = format!(
                "+{} more · [panel.computers] hosts picks them",
                list.len() - shown
            );
            for (n, l) in wrap(&more, chars, 3).iter().enumerate() {
                let b = self.baseline(cy + row_h * n as i16, i32::from(row_h));
                self.text(cx, b, l, pal.muted, pal.surface, chars as i32 * adv)?;
            }
        }
        Ok(y + height + gap)
    }

    /// The search field's hint: what typing finds.
    fn hint(&self) -> String {
        let what: Vec<&str> = [
            ("apps", "apps"),
            ("files", "files"),
            ("settings", "settings"),
            ("keys", "key bindings"),
            ("computers", "computers"),
            ("manual", "the manual"),
        ]
        .iter()
        .filter(|(g, _)| self.finds(g))
        .map(|(_, w)| *w)
        .collect();
        match what.split_last() {
            None => "Tab moves · Enter chooses · Esc closes".into(),
            Some((last, [])) => format!("Type to find {last} · Tab moves · Esc closes"),
            Some((last, rest)) => {
                format!("Type to search {} and {last} · Esc closes", rest.join(", "))
            }
        }
    }

    /// The clipboard card: the newest entries, each on up to two lines; a
    /// click copies one back.
    fn draw_clips(&mut self, x: i16, y: i16, w: i32, bottom: i16) -> Result<()> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance).max(1);
        let clips = match self.clips.clone() {
            None => return self.lines(x, y, w, bottom, &[("…".into(), pal.muted)]),
            Some(c) if c.is_empty() => {
                let text = "Nothing copied yet. NIRUCLIPBOARD keeps what you copy here.";
                return self.lines(x, y, w, bottom, &[(text.into(), pal.muted)]);
            }
            Some(c) => c,
        };
        let row_h = lh + 2;
        let chars = (w / adv).max(4) as usize;
        let mut ry = i32::from(y);
        for (k, item) in clips.iter().enumerate() {
            let lines = wrap(&item.name, chars, 2);
            let rh = lines.len() as i32 * row_h + 3;
            if ry + rh > i32::from(bottom) {
                break;
            }
            let hit = Hit::Clip(k);
            let bg = self.row_box(rect(x - 4, ry as i16, w + 8, rh - 1), hit, pal.surface)?;
            for (n, l) in lines.iter().enumerate() {
                let b = self.baseline((ry + 1 + n as i32 * row_h) as i16, row_h);
                self.text(x, b, l, pal.foreground, bg, chars as i32 * adv)?;
            }
            self.hits.push((rect(x - 4, ry as i16, w + 8, rh - 1), hit));
            self.targets.push(hit);
            ry += rh;
        }
        Ok(())
    }

    /// The notifications card: who and what on one line, the text below.
    fn notice_lines(&self) -> Vec<(String, u32)> {
        let pal = self.cfg.palette;
        match &self.notices {
            None => vec![("…".into(), pal.muted)],
            Some(Err(e)) => vec![(
                format!("{e}: notifications show here when it does"),
                pal.muted,
            )],
            Some(Ok(n)) if n.list.is_empty() => {
                vec![(
                    "No notifications since NIRUNOTIFY started".into(),
                    pal.muted,
                )]
            }
            Some(Ok(n)) => n
                .list
                .iter()
                .flat_map(|m| {
                    let head = match (m.app.is_empty(), m.summary.is_empty()) {
                        (false, false) => format!("{}: {}", m.app, m.summary),
                        (true, _) => m.summary.clone(),
                        (false, true) => m.app.clone(),
                    };
                    let mut out = vec![(head, pal.foreground)];
                    if !m.body.is_empty() {
                        out.push((m.body.clone(), pal.muted));
                    }
                    out
                })
                .collect(),
        }
    }

    /// A row that can be chosen: its background, an accent border when it
    /// has the focus (the next target). Returns the colour drawn under it.
    fn row_box(&mut self, row: Rectangle, hit: Hit, base: u32) -> Result<u32> {
        let pal = self.cfg.palette;
        let focused = self.targets.len() == self.focus;
        let bg = if focused || self.hover == Some(hit) {
            pal.selected
        } else {
            base
        };
        if focused {
            self.fill(row, pal.accent)?;
            self.fill(
                rect(
                    row.x + 1,
                    row.y + 1,
                    i32::from(row.width) - 2,
                    i32::from(row.height) - 2,
                ),
                bg,
            )?;
        } else {
            self.fill(row, bg)?;
        }
        Ok(bg)
    }

    /// A box for results: the border and the card's surface.
    fn boxed(&self, r: Rectangle) -> Result<()> {
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
        )
    }

    /// A group's title band inside a results box. Returns the y below it.
    fn band(&mut self, r: Rectangle, y: i32, title: &str) -> Result<i32> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let band = rect(r.x + 1, y as i16, i32::from(r.width) - 2, lh + 6);
        self.fill(band, pal.selected)?;
        let b = self.baseline(y as i16, lh + 6);
        self.text(
            r.x + 8,
            b,
            title,
            pal.warning,
            pal.selected,
            i32::from(r.width) - 16,
        )?;
        Ok(y + lh + 10)
    }

    /// The room of the results when they have it to themselves: the
    /// results while searching, else a line that says what comes there.
    fn draw_results_room(&mut self, y: i32, area: i32, pad: i32, searching: bool) -> Result<i32> {
        if searching {
            return self.draw_results(y, area, pad);
        }
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let b = self.baseline(y as i16, lh * 2);
        self.text(
            pad as i16,
            b,
            "Settings, key bindings, files and more show here while you type.",
            pal.muted,
            pal.background,
            i32::from(self.w) - pad * 2,
        )?;
        Ok(y + area)
    }

    /// The search results in `area`: the groups found at once on the left
    /// (settings and actions, key bindings, computers, the manual), the
    /// files on the right as they come in.
    fn draw_results(&mut self, y: i32, area: i32, pad: i32) -> Result<i32> {
        let gap = pad / 2;
        let w = i32::from(self.w) - pad * 2;
        let r = rect(pad as i16, y as i16, w, area - gap);
        let files = self.finds("files");
        let near: Vec<Group> = Group::NEAR
            .iter()
            .copied()
            .filter(|g| self.finds(g.name()))
            .collect();
        let two = files && !near.is_empty();
        let left_w = if two { (w - gap) * 11 / 20 } else { w };
        if !near.is_empty() {
            self.draw_near(rect(r.x, r.y, left_w, i32::from(r.height)), &near)?;
        }
        if files {
            let (fx, fw) = if two {
                (r.x + (left_w + gap) as i16, w - left_w - gap)
            } else {
                (r.x, w)
            };
            self.draw_files(rect(fx, r.y, fw, i32::from(r.height)))?;
        }
        Ok(y + area)
    }

    /// One result: the hit, a lead (keys, a dot) in its colour, the name
    /// in its colour and what it is.
    fn result_rows(&self, g: Group) -> Vec<(Hit, String, u32, String, String, u32)> {
        let pal = self.cfg.palette;
        let source = self.source(g);
        let now = data::now();
        source
            .shown
            .iter()
            .filter_map(|&i| {
                let item = source.get(i)?;
                Some(match g {
                    Group::Keys => (
                        Hit::Key(i),
                        key_label(item),
                        pal.warning,
                        item.name.clone(),
                        String::new(),
                        if matches!(item.target, Target::Info) {
                            pal.muted
                        } else {
                            pal.foreground
                        },
                    ),
                    Group::Computers => {
                        let d = self.computer(i)?;
                        let (detail, _) = computer_detail(d, now);
                        let dot = if d.online { pal.success } else { pal.muted };
                        (
                            Hit::Computer(i),
                            "●".into(),
                            dot,
                            d.name.clone(),
                            detail,
                            pal.foreground,
                        )
                    }
                    Group::Settings => (
                        Hit::Setting(i),
                        String::new(),
                        pal.muted,
                        item.name.clone(),
                        item.detail.clone(),
                        pal.foreground,
                    ),
                    _ => (
                        Hit::Manual(i),
                        String::new(),
                        pal.muted,
                        item.name.clone(),
                        item.detail.clone(),
                        pal.foreground,
                    ),
                })
            })
            .collect()
    }

    fn draw_near(&mut self, r: Rectangle, groups: &[Group]) -> Result<()> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance).max(1);
        self.boxed(r)?;
        let x = r.x + 8;
        let inner = i32::from(r.width) - 16;
        let chars = (inner / adv).max(8) as usize;
        let top = i32::from(r.y) + 1;
        let bottom = i32::from(r.y) + i32::from(r.height) - 4;
        let row_h = lh + 2;
        // Each group's rows, laid out: (lead width, lines per row).
        let mut laid = Vec::new();
        for &g in groups {
            let rows = self.result_rows(g);
            let lead = rows
                .iter()
                .map(|r| r.1.chars().count())
                .max()
                .unwrap_or(0)
                .min(18);
            let off = if lead > 0 { lead + 2 } else { 0 };
            let lines: Vec<Vec<(String, String)>> = rows
                .iter()
                .map(|r| search::row_lines(&r.3, &r.4, chars.saturating_sub(off).max(6)))
                .collect();
            laid.push((g, rows, off, lines));
        }
        let costs: Vec<Vec<i32>> = laid
            .iter()
            .map(|(_, _, _, lines)| lines.iter().map(|l| l.len() as i32 * row_h + 4).collect())
            .collect();
        let take = search::allocate(&costs, lh + 10, bottom - top);
        if take.iter().all(|&n| n == 0) {
            let loading = groups.iter().any(|&g| self.source(g).loading());
            let text = if loading {
                "Searching…".to_string()
            } else {
                "Nothing among the key bindings, settings, computers or the manual matches."
                    .to_string()
            };
            self.lines(
                x,
                (top + 6) as i16,
                inner,
                bottom as i16,
                &[(text, pal.muted)],
            )?;
            return Ok(());
        }
        let mut y = top;
        for ((g, rows, off, lines), n) in laid.iter().zip(take) {
            if n == 0 {
                continue;
            }
            let found = rows.len();
            let more = if found >= g.cap() { "+" } else { "" };
            let title = if n < found {
                format!("{} · {n} of {found}{more}", g.title())
            } else {
                format!("{} · {found}", g.title())
            };
            y = self.band(r, y, &title)?;
            for (row, lines) in rows.iter().zip(lines).take(n) {
                let (hit, lead, lead_colour, _, _, colour) = row;
                let rh = lines.len() as i32 * row_h + 4;
                let bg =
                    self.row_box(rect(x - 4, y as i16, inner + 8, rh - 2), *hit, pal.surface)?;
                if !lead.is_empty() {
                    let b = self.baseline(y as i16 + 1, row_h);
                    let shown = wrap(lead, off.saturating_sub(2).max(1), 1).remove(0);
                    self.text(x, b, &shown, *lead_colour, bg, *off as i32 * adv)?;
                }
                let nx = x + (*off as i32 * adv) as i16;
                for (k, (name, detail)) in lines.iter().enumerate() {
                    let b = self.baseline((y + 1 + k as i32 * row_h) as i16, row_h);
                    let used = name.chars().count();
                    if !name.is_empty() {
                        self.text(nx, b, name, *colour, bg, used as i32 * adv + adv)?;
                    }
                    if !detail.is_empty() {
                        let dx = if used > 0 { used + 2 } else { 0 };
                        self.text(
                            nx + (dx as i32 * adv) as i16,
                            b,
                            detail,
                            pal.muted,
                            bg,
                            inner - (*off + dx) as i32 * adv,
                        )?;
                    }
                }
                self.hits
                    .push((rect(x - 4, y as i16, inner + 8, rh - 2), *hit));
                self.targets.push(*hit);
                y += rh;
            }
            y += 2;
        }
        Ok(())
    }

    /// The files the search found, as they come in: the kind, the name
    /// and the folder it is in; at the bottom how they were found.
    fn draw_files(&mut self, r: Rectangle) -> Result<()> {
        let pal = self.cfg.palette;
        let lh = i32::from(self.r.line_height);
        let adv = i32::from(self.r.advance).max(1);
        self.boxed(r)?;
        let x = r.x + 8;
        let inner = i32::from(r.width) - 16;
        let chars = (inner / adv).max(8) as usize;
        let row_h = lh + 2;
        let top = i32::from(r.y) + 1;
        let bottom = i32::from(r.y) + i32::from(r.height) - 4;
        let busy = self.files.rx.is_some();
        let rows: Vec<Item> = self.files.rows.clone();
        let found = rows
            .iter()
            .filter(|i| !matches!(i.target, Target::Info))
            .count();
        let title = match (busy, found) {
            (true, 0) => "FILES · searching…".to_string(),
            (true, n) => format!("FILES · {n} · searching…"),
            (false, n) => format!("FILES · {n}"),
        };
        let mut y = self.band(r, top, &title)?;
        let short = self.query.trim().chars().count() < 2;
        // The last line says how the files were found.
        let note = if short {
            String::new()
        } else {
            self.files.note.clone()
        };
        let end = if note.is_empty() {
            bottom
        } else {
            bottom - row_h
        };
        if short || rows.is_empty() {
            let text = if short {
                "Type one more letter to search files."
            } else if busy {
                "Searching your files…"
            } else {
                "No file matches."
            };
            self.lines(x, y as i16, inner, end as i16, &[(text.into(), pal.muted)])?;
        }
        let lead = rows
            .iter()
            .map(|i| i.group.chars().count())
            .max()
            .unwrap_or(0)
            .min(10);
        let off = if lead > 0 { lead + 2 } else { 0 };
        for (k, item) in rows.iter().enumerate() {
            if matches!(item.target, Target::Info) {
                self.lines(
                    x,
                    y as i16,
                    inner,
                    end as i16,
                    &[(item.name.clone(), pal.muted)],
                )?;
                y += row_h;
                continue;
            }
            let lines =
                search::row_lines(&item.name, &item.detail, chars.saturating_sub(off).max(6));
            let rh = lines.len() as i32 * row_h + 4;
            if y + rh > end {
                break;
            }
            let hit = Hit::File(k);
            let bg = self.row_box(rect(x - 4, y as i16, inner + 8, rh - 2), hit, pal.surface)?;
            let b = self.baseline(y as i16 + 1, row_h);
            let kind = wrap(&item.group, lead.max(1), 1).remove(0);
            self.text(x, b, &kind, pal.warning, bg, off as i32 * adv)?;
            let nx = x + (off as i32 * adv) as i16;
            for (n, (name, detail)) in lines.iter().enumerate() {
                let b = self.baseline((y + 1 + n as i32 * row_h) as i16, row_h);
                let used = name.chars().count();
                if !name.is_empty() {
                    self.text(nx, b, name, pal.foreground, bg, used as i32 * adv + adv)?;
                }
                if !detail.is_empty() {
                    let dx = if used > 0 { used + 2 } else { 0 };
                    self.text(
                        nx + (dx as i32 * adv) as i16,
                        b,
                        detail,
                        pal.muted,
                        bg,
                        inner - (off + dx) as i32 * adv,
                    )?;
                }
            }
            self.hits
                .push((rect(x - 4, y as i16, inner + 8, rh - 2), hit));
            self.targets.push(hit);
            y += rh;
        }
        if !note.is_empty() {
            let b = self.baseline((bottom - row_h) as i16, row_h);
            let shown = wrap(&note, chars, 1).remove(0);
            self.text(x, b, &shown, pal.muted, pal.surface, inner)?;
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
        if !matches!(hit, Hit::Key(_)) {
            self.confirm_key = None;
        }
        match hit {
            Hit::Computer(i) => self.on_computer(i, ComputerAction::Ssh),
            Hit::Key(i) => {
                let Some(item) = self.keys.get(i).cloned() else {
                    return true;
                };
                match item.target.clone() {
                    Target::RunKey(keys) => {
                        if risky_key(&item) && self.confirm_key != Some(i) {
                            self.confirm_key = Some(i);
                            return true;
                        }
                        // Close first: the binding acts on the window that
                        // had the focus before the panel, or opens the panel.
                        self.close();
                        let _ = self.c.get_input_focus().map(|r| r.reply());
                        let argv = ["niructl".to_string(), "keys".into(), "run".into(), keys];
                        if let Err(e) = spawn_command(&argv, None, false, &self.cfg) {
                            eprintln!("nirupanel: {e}");
                        }
                        false
                    }
                    Target::Spawn { argv, .. } => {
                        self.close();
                        if let Err(e) = spawn_command(&argv, None, false, &self.cfg) {
                            eprintln!("nirupanel: {e}");
                        }
                        false
                    }
                    _ => {
                        self.note = format!(
                            "{}: {} (shown only, it cannot be run from here)",
                            key_label(&item),
                            item.name
                        );
                        true
                    }
                }
            }
            Hit::Setting(i) => match self.settings.get(i).cloned() {
                Some(item) => self.run_item(&item),
                None => true,
            },
            Hit::Manual(i) => match self.manual.get(i).cloned() {
                Some(item) => self.run_item(&item),
                None => true,
            },
            Hit::File(i) => self.on_file(i, FileAction::Open),
            Hit::Clip(i) => match self.clips.as_ref().and_then(|c| c.get(i)).cloned() {
                Some(item) => self.run_item(&item),
                None => true,
            },
            Hit::Clipboard => self.open_view("clipboard"),
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
            // NIRUSYS, the NIRUX system monitor extra, when it is installed.
            Hit::System if data::on_path("nirusys") => self.spawn(&["nirusys"]),
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

    /// Do something with a computer. `false` closes the panel. SSH runs
    /// only in a terminal the user sees: there are no keys between the
    /// computers, so it asks for a password there.
    fn on_computer(&mut self, i: usize, what: ComputerAction) -> bool {
        let Some(d) = self.computer(i).cloned() else {
            return true;
        };
        if !safe_host(&d.target) {
            self.note = format!("{} has a name ssh cannot take", d.name);
            return true;
        }
        let copy = |text: &str| match copy_to_clipboard(text) {
            Ok(()) => format!("Copied {text}"),
            Err(e) => e,
        };
        match what {
            ComputerAction::Ssh => {
                let argv = vec!["ssh".to_string(), d.target.clone()];
                match spawn_terminal_command(&argv, true, &self.cfg) {
                    Ok(()) => false,
                    Err(e) => {
                        self.note = e;
                        true
                    }
                }
            }
            ComputerAction::Files => {
                let uri = format!("sftp://{}/", d.target);
                self.spawn(&["xdg-open", &uri])
            }
            ComputerAction::CopyIp => {
                self.note = if d.ip.is_empty() {
                    format!("{} has no Tailscale address", d.name)
                } else {
                    copy(&d.ip)
                };
                true
            }
            ComputerAction::CopyName => {
                self.note = copy(&d.name);
                true
            }
        }
    }

    /// Do what a row of the launcher does, as the launcher would. `false`
    /// closes the panel.
    fn run_item(&mut self, item: &Item) -> bool {
        let done = match item.target.clone() {
            Target::Spawn {
                argv,
                terminal,
                path,
            } => spawn_command(&argv, path.as_deref(), terminal, &self.cfg),
            Target::Terminal { argv, hold } => spawn_terminal_command(&argv, hold, &self.cfg),
            Target::Mode(mode) => return self.open_view(&mode),
            Target::Copy(text) => copy_to_clipboard(&text),
            Target::RunKey(keys) => {
                self.close();
                let argv = ["niructl".to_string(), "keys".into(), "run".into(), keys];
                spawn_command(&argv, None, false, &self.cfg)
            }
            _ => {
                self.note = format!("{}: open it in the launcher", item.name);
                return true;
            }
        };
        match done {
            Ok(()) => {
                record_usage(&item.key);
                false
            }
            Err(e) => {
                self.note = e;
                true
            }
        }
    }

    /// Open a view of the launcher (`nirulauncher --VIEW`). The panel lets
    /// go of the keyboard first, so the launcher can take it.
    fn open_view(&mut self, mode: &str) -> bool {
        let Some(flag) = search::view_flag(mode) else {
            self.note = format!("The launcher has no view {mode}");
            return true;
        };
        self.close();
        if let Err(e) = spawn_command(&["nirulauncher".into(), flag], None, false, &self.cfg) {
            eprintln!("nirupanel: {e}");
        }
        false
    }

    /// Open a file the search found, the folder it is in, or copy its path.
    /// `false` closes the panel.
    fn on_file(&mut self, i: usize, what: FileAction) -> bool {
        let Some(item) = self.files.rows.get(i).cloned() else {
            return true;
        };
        let Some(path) = item.key.strip_prefix(crate::modes::files::KEY_PREFIX) else {
            return true;
        };
        match what {
            FileAction::Open => self.run_item(&item),
            FileAction::Folder => {
                let folder = crate::modes::files::parent_of(path);
                self.spawn(&["xdg-open", &folder])
            }
            FileAction::CopyPath => {
                self.note = match copy_to_clipboard(path) {
                    Ok(()) => format!("Copied {path}"),
                    Err(e) => e,
                };
                true
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

    /// Up and down move a row in a grid (the apps, the computers) or a
    /// line among the key bindings, otherwise to the group before or after;
    /// left and right go one step.
    fn step(&mut self, sym: u32) {
        let n = self.targets.len();
        if n == 0 {
            return;
        }
        self.moved = true;
        let lh = i32::from(self.r.line_height);
        let isz = Self::icon_size(lh);
        let tw = (isz + lh * 2).max(i32::from(self.r.advance) * 11);
        let app_cols = ((i32::from(self.w) - lh * 2) / tw).max(1) as usize;
        let f = self.focus.min(n - 1);
        let searching = self.showing_results();
        // Left and right cross the two columns of search results.
        if matches!(sym, 0xff51 | 0xff53) {
            let here = self.targets[f];
            if is_result(here) || (searching && matches!(here, Hit::Computer(_))) {
                if let Some(k) = self.across(here, sym == 0xff53) {
                    self.focus = k;
                    return;
                }
            }
        }
        let groups: Vec<u8> = self.targets.iter().map(|t| group_of(*t)).collect();
        self.focus = step_in(&groups, f, sym, |g| match g {
            0 => app_cols,
            3 if searching => 1,
            3 => self.computer_cols.max(1),
            4 | 7..=10 => 1,
            _ => 0,
        });
    }

    /// The result in the other column nearest to `from`, to the right or
    /// to the left.
    fn across(&self, from: Hit, right: bool) -> Option<usize> {
        let at = |h: Hit| self.hits.iter().find(|(_, x)| *x == h).map(|(r, _)| *r);
        let a = at(from)?;
        let mid = |r: Rectangle| i32::from(r.y) + i32::from(r.height) / 2;
        self.targets
            .iter()
            .enumerate()
            .filter(|(_, h)| is_result(**h) || matches!(h, Hit::Computer(_)))
            .filter_map(|(k, h)| Some((k, at(*h)?)))
            .filter(|(_, r)| {
                if right {
                    r.x >= a.x + a.width as i16
                } else {
                    r.x + r.width as i16 <= a.x
                }
            })
            .min_by_key(|(_, r)| (mid(*r) - mid(a)).abs())
            .map(|(k, _)| k)
    }

    fn key(&mut self, e: KeyPressEvent) -> Result<bool> {
        let sym = self.keymap.symbol(e.detail, e.state);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        let shift = e.state.contains(KeyButMask::SHIFT);
        self.note.clear();
        let focused = self.targets.get(self.focus).copied();
        match sym {
            // Ctrl+C copies a computer's address, Ctrl+Shift+C its name.
            0x63 | 0x43 if ctrl => match focused {
                Some(Hit::Computer(i)) => {
                    let what = if shift {
                        ComputerAction::CopyName
                    } else {
                        ComputerAction::CopyIp
                    };
                    self.on_computer(i, what);
                }
                Some(Hit::File(i)) => {
                    self.on_file(i, FileAction::CopyPath);
                }
                _ => {}
            },
            0xff0d | 0xff8d if shift && matches!(focused, Some(Hit::File(_))) => {
                if let Some(Hit::File(i)) = focused {
                    if !self.on_file(i, FileAction::Folder) {
                        return Ok(false);
                    }
                }
            }
            0xff0d | 0xff8d if shift && matches!(focused, Some(Hit::Computer(_))) => {
                if let Some(Hit::Computer(i)) = focused {
                    if !self.on_computer(i, ComputerAction::Files) {
                        return Ok(false);
                    }
                }
            }
            0xff1b => {
                if self.confirm.take().is_some() || self.confirm_key.take().is_some() {
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.refilter();
                } else {
                    return Ok(false);
                }
            }
            0xff0d | 0xff8d => {
                // While searching, Enter takes what the search found (an
                // app, a key binding), or the target moved to by hand.
                let searching = !self.query.trim().is_empty();
                let hit = focused.filter(|h| {
                    !searching
                        || self.moved
                        || matches!(h, Hit::App(_))
                        || is_result(*h)
                        || (self.showing_results() && matches!(h, Hit::Computer(_)))
                });
                if let Some(hit) = hit {
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
            0xff09 => {
                self.moved = true;
                self.focus = (self.focus + 1) % self.targets.len().max(1);
            }
            0xfe20 => {
                self.moved = true;
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
                    Msg::Computers(list) => {
                        if let Ok(l) = &list {
                            self.computer_rows_found.items = Some(search::computer_items(l));
                        }
                        self.computers = Some(list);
                        self.filter_results();
                    }
                    Msg::Rows(g, rows) => {
                        match g {
                            Group::Keys => self.keys.items = Some(rows),
                            Group::Settings => self.settings.items = Some(rows),
                            _ => self.manual.items = Some(rows),
                        }
                        self.filter_results();
                        if self.debug && !self.query.trim().is_empty() {
                            eprintln!(
                                "nirupanel: {} ready {} ms after the search changed",
                                g.name(),
                                self.searched.elapsed().as_millis()
                            );
                        }
                    }
                    Msg::Clips(c) => self.clips = Some(c),
                    Msg::Notices(n) => self.notices = Some(n),
                    Msg::Icon(key, p) => {
                        self.pending.remove(&key);
                        self.pictures.insert(key, p);
                    }
                }
            }
            if self.files.poll() {
                dirty = true;
                if self.debug {
                    eprintln!(
                        "nirupanel: files for '{}' shown {} ms after the search changed ({})",
                        self.query.trim(),
                        self.searched.elapsed().as_millis(),
                        self.files.note
                    );
                }
            }
            if self.files.partial && self.files.rx.is_none() && !crate::modes::files::walk_running()
            {
                self.files.again(&self.cfg.files);
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
                        let hit = self
                            .hits
                            .iter()
                            .rev()
                            .find(|(r, _)| inside(*r, e.event_x, e.event_y))
                            .map(|(_, h)| *h);
                        // A computer: the middle button opens its files, the
                        // right one copies its address.
                        let other = match (e.detail, hit) {
                            (2, Some(Hit::Computer(i))) => Some((i, ComputerAction::Files)),
                            (3, Some(Hit::Computer(i))) => Some((i, ComputerAction::CopyIp)),
                            // A file: the middle button opens its folder,
                            // the right one copies its path.
                            (2 | 3, Some(Hit::File(i))) => {
                                let what = if e.detail == 2 {
                                    FileAction::Folder
                                } else {
                                    FileAction::CopyPath
                                };
                                if !self.on_file(i, what) {
                                    return Ok(());
                                }
                                self.draw()?;
                                continue;
                            }
                            (1, _) => None,
                            _ => continue,
                        };
                        if let Some(hit) = hit {
                            if let Some(k) = self.targets.iter().position(|t| *t == hit) {
                                self.focus = k;
                                self.moved = true;
                            }
                            self.note.clear();
                            let open = match other {
                                Some((i, what)) => self.on_computer(i, what),
                                None => self.activate(hit),
                            };
                            if !open {
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
            // While a search is out, look for its answer often.
            let waiting = self.files.rx.is_some()
                || [&self.keys, &self.settings, &self.manual]
                    .iter()
                    .any(|s| s.loading());
            let wait = if waiting { 15 } else { 250 };
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut pfd, 1, wait) };
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
        let c =
            cfg("[panel]\nmodules = [\"power\", \"apps\", \"nonsense\", \"apps\"]\napp_rows = 9");
        assert_eq!(c.panel.modules, ["power", "apps"]);
        assert_eq!(c.panel.app_rows, 4);
        assert_eq!(cfg("[panel]\nmodules = []").panel.modules, MODULES);
        let c = cfg(
            "[panel.computers]\nhosts = [\"othala\", \" \", \"mac\"]\nhide = [\"box\"]\nrows = 0",
        );
        assert_eq!(c.panel.computer_hosts, ["othala", "mac"]);
        assert_eq!(c.panel.computer_hide, ["box"]);
        assert_eq!(c.panel.computer_rows, 1);
        assert!(cfg("").panel.computer_hosts.is_empty());
        assert_eq!(cfg("").panel.computer_rows, 2);
        assert_eq!(cfg("").panel.search, SEARCH);
        let c = cfg("[panel]\nsearch = [\"Files\", \"apps\", \"web\", \"files\"]");
        assert_eq!(c.panel.search, ["files", "apps"]);
        assert_eq!(cfg("[panel]\nsearch = []").panel.search, ["apps"]);
    }

    #[test]
    fn the_search_finds_what_its_modules_allow() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = v(&SEARCH);
        assert!(has_results(&v(&["apps"]), &all));
        assert!(finds(&v(&["apps"]), &all, "files"));
        // Key bindings and computers need their modules.
        assert!(!finds(&v(&["apps"]), &all, "keys"));
        assert!(!finds(&v(&["apps"]), &all, "computers"));
        assert!(finds(&v(&["keys"]), &all, "keys"));
        assert!(!has_results(&v(&["apps", "keys"]), &v(&["apps"])));
        assert!(has_results(&v(&["keys"]), &v(&["keys"])));
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
        let h = |m: &[String], apps, comps, results| {
            Panel::height_for(m, apps, comps, 19, 9, 1000, results)
        };
        let tall = h(&all, 2, 2, true);
        let short = h(&all, 1, 2, true);
        assert!(tall > short);
        assert!(h(&all, 1, 1, true) < short);
        // With cards, the results take their place: no room of their own.
        assert_eq!(h(&all, 2, 2, false), tall);
        let none: Vec<String> = vec!["power".into()];
        assert!(h(&none, 2, 2, false) < short);
        // Search results get room of their own without cards or computers.
        assert_eq!(
            h(&none, 2, 2, true),
            h(&none, 2, 2, false) + Panel::results_height(19)
        );
        // Six cards where four fit: two rows of three.
        assert_eq!(Panel::card_columns(1000, 9, 4), 4);
        assert_eq!(Panel::card_columns(1000, 9, 6), 3);
        assert_eq!(Panel::card_columns(1000, 9, 5), 3);
        assert_eq!(Panel::card_columns(720, 9, 4), 2);
        assert_eq!(Panel::card_columns(400, 9, 4), 1);
        assert_eq!(Panel::card_columns(400, 9, 0), 1);
        // A card holds six lines of text below its title, four above its
        // buttons.
        for lh in [15, 19, 24, 30] {
            assert!(
                (Panel::card_height(lh) - 4 - (lh + 10)) / (lh + 2) >= 6,
                "{lh}"
            );
            assert!((Panel::card_height(lh) - lh * 2 - 8 - (lh + 10)) / (lh + 2) >= 4);
        }
        let lh = 19;
        // An app's icon and two lines of its name fit in its tile.
        assert!(6 + Panel::icon_size(lh) + 4 + lh * 2 <= Panel::tile_height(lh) - 4);
    }

    fn key_item(name: &str, detail: &str, keywords: &str, target: Target) -> Item {
        Item {
            key: format!("keys:NIRUWM:{detail}"),
            group: "Session".into(),
            name: name.into(),
            detail: detail.into(),
            keywords: keywords.into(),
            favorite: false,
            target,
        }
    }

    #[test]
    fn key_bindings_that_end_or_close_things_ask_first() {
        let run = || Target::RunKey("super+x".into());
        let close = key_item(
            "Close the focused window",
            "Super+Q",
            "super+q Super+Q close NIRUWM #k40-71",
            run(),
        );
        let quit = key_item(
            "End the session",
            "Super+Ctrl+Shift+Q",
            "super+ctrl+shift+q x quit NIRUWM",
            run(),
        );
        let sleep = key_item(
            "Lock, then suspend the computer",
            "Super+Shift+Escape",
            "suspend",
            run(),
        );
        let term = key_item(
            "Open a terminal",
            "Super+Enter · conflict",
            "super+Return Super+Enter terminal NIRUWM",
            run(),
        );
        let lock = key_item(
            "Lock the screen",
            "Super+Escape",
            "super+Escape lock NIRUWM",
            run(),
        );
        assert!(risky_key(&close) && risky_key(&quit) && risky_key(&sleep));
        assert!(!risky_key(&term) && !risky_key(&lock));
        assert_eq!(key_label(&term), "Super+Enter");
    }

    #[test]
    fn key_bindings_are_found_by_words_in_any_order() {
        let sheet = r#"{"version":1,"entries":[
          {"keys":"super+Return","label":"Super+Enter","mods":64,"keysym":65293,"action":"terminal","run":null,
           "description":"Open a terminal","group":"Apps and launchers","source":"niruwm","app":null,"conflicts":[]},
          {"keys":"super+shift+s","label":"Super+Shift+S","mods":65,"keysym":115,"action":"screenshot region","run":null,
           "description":"Skärmbild of a region","group":"Screenshots and color","source":"niruwm","app":null,"conflicts":[]},
          {"keys":"super+space","label":"Super+Space","mods":64,"keysym":32,"action":"spawn nirulauncher --panel","run":null,
           "description":"Open NIRUPANEL","group":"Apps and launchers","source":"niruwm","app":null,"conflicts":[]}]}"#;
        let items = crate::modes::keys::parse_sheet(sheet).unwrap();
        let usage = usage::Usage::default();
        let names = |q: &str| -> Vec<String> {
            ranked_for_mode(&items, q, &usage, "keys")
                .order
                .into_iter()
                .map(|i| items[i].name.clone())
                .collect()
        };
        assert_eq!(names("terminal open"), ["Open a terminal"]);
        assert_eq!(names("TERMINAL"), ["Open a terminal"]);
        // Diacritics either way.
        assert_eq!(names("skarmbild region"), ["Skärmbild of a region"]);
        assert_eq!(names("REGION skärm"), ["Skärmbild of a region"]);
        // By the key itself.
        assert_eq!(names("super+space")[0], "Open NIRUPANEL");
        assert!(names("nothing like this").is_empty());
    }

    #[test]
    fn arrows_move_in_grids_and_between_groups() {
        // Six apps in rows of four, three computers in rows of two, two keys.
        let g = [0, 0, 0, 0, 0, 0, 3, 3, 3, 4, 4];
        let cols = |g: u8| match g {
            0 => 4,
            3 => 2,
            4 => 1,
            _ => 0,
        };
        let (up, down, left, right) = (0xff52, 0xff54, 0xff51, 0xff53);
        assert_eq!(step_in(&g, 1, down, cols), 5);
        assert_eq!(
            step_in(&g, 3, down, cols),
            6,
            "below the last row: the next group"
        );
        assert_eq!(step_in(&g, 5, up, cols), 1);
        assert_eq!(step_in(&g, 1, up, cols), 1, "nothing above");
        assert_eq!(step_in(&g, 6, down, cols), 8);
        assert_eq!(step_in(&g, 7, down, cols), 9);
        assert_eq!(step_in(&g, 8, up, cols), 6);
        assert_eq!(step_in(&g, 6, up, cols), 0, "the start of the apps");
        assert_eq!(step_in(&g, 9, down, cols), 10);
        assert_eq!(step_in(&g, 10, down, cols), 10);
        assert_eq!(step_in(&g, 0, left, cols), 0);
        assert_eq!(step_in(&g, 10, right, cols), 10);
        // Not a grid: Up and Down go to the next group.
        let flat = [5, 5, 5, 6, 6];
        assert_eq!(step_in(&flat, 1, down, cols), 3);
        assert_eq!(step_in(&flat, 4, up, cols), 0);
    }

    #[test]
    fn computers_say_what_they_are() {
        let d = |os: &str| Device {
            name: "othala".into(),
            host_name: "othala".into(),
            target: "othala".into(),
            ip: "100.64.0.2".into(),
            os: os.into(),
            online: true,
            this: false,
            mine: true,
            last_seen: None,
            exit_node: false,
            direct: Some(true),
            relay: "ams".into(),
        };
        assert_eq!(computer_detail(&d("linux"), 0).0, "Linux · online · direct");
        let relayed = Device {
            direct: Some(false),
            exit_node: true,
            ..d("macOS")
        };
        assert_eq!(
            computer_detail(&relayed, 0).0.replace(wrap::NBSP, " "),
            "macOS · online · relay ams · exit node"
        );
        let off = Device {
            online: false,
            direct: None,
            last_seen: Some(1000),
            ..d("windows")
        };
        assert_eq!(
            computer_detail(&off, 1000 + 7200)
                .0
                .replace(wrap::NBSP, " "),
            "Windows · offline · seen 2 h ago"
        );
        let me = Device {
            this: true,
            ..d("linux")
        };
        assert_eq!(
            computer_detail(&me, 0),
            ("this computer · Linux".into(), true)
        );
        assert!(safe_host("othala") && safe_host("100.64.0.2") && safe_host("fd7a::1"));
        assert!(!safe_host("-oProxyCommand=x") && !safe_host("a b") && !safe_host(""));
        assert!(!safe_host("x;rm"));
    }
}
