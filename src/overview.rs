//! Workspace overview (`--overview`, Super+E in NIRUWM).
//!
//! NIRUWM writes `state.json` and one small binary PPM per tag to
//! `$XDG_RUNTIME_DIR/niruwm/overview/` and then starts this view. Every
//! monitor gets a section with one card per tag: its thumbnail (or a
//! placeholder), the tag number and the window titles. Picking a card runs
//! `niructl do goto MONITOR TAG [WINDOW]`; NIRUWM does the switching.
//!
//! No compositor and no animation: the thumbnails are scaled once with a box
//! filter and the frame is drawn through the same double-buffered renderer as
//! the other views.

use crate::config::Config;
use crate::input::Keymap;
use crate::render::Renderer;
use crate::ui::{intern, monitor_at, point, set_utf8, wait_until_viewable};
use crate::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// The `state.json` version this view understands.
const STATE_VERSION: u32 = 1;
/// Largest thumbnail side NIRUWM writes; a card never draws one larger.
const MAX_THUMB: u16 = 384;
/// Cards per row at most.
const MAX_COLS: usize = 5;
/// A state older than this says so in the header.
const STALE_SECS: u64 = 600;

#[derive(Debug, Deserialize)]
pub(crate) struct State {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) monitors: Vec<Monitor>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Monitor {
    pub(crate) index: u32,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) x: i32,
    #[serde(default)]
    pub(crate) y: i32,
    #[serde(default)]
    pub(crate) width: u32,
    #[serde(default)]
    pub(crate) height: u32,
    #[serde(default)]
    pub(crate) focused: bool,
    #[serde(default)]
    pub(crate) current_tag: u32,
    #[serde(default)]
    pub(crate) tags: Vec<Tag>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Tag {
    pub(crate) tag: u32,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) thumbnail: String,
    #[serde(default)]
    pub(crate) windows: Vec<Win>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Win {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) class: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) focused: bool,
}

impl Win {
    fn label(&self) -> String {
        let title = self.title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            self.class.clone()
        } else {
            title
        }
    }
}

/// `$XDG_RUNTIME_DIR/niruwm/overview`, or None without a runtime directory
/// (NIRUWM writes nothing then).
pub(crate) fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(base).join("niruwm/overview"))
}

/// Parse `state.json` and tidy it: monitors and tags in order, the current
/// tag always present, monitors without a size dropped.
pub(crate) fn parse_state(text: &str) -> std::result::Result<State, String> {
    let mut state: State =
        serde_json::from_str(text).map_err(|e| format!("cannot read state.json: {e}"))?;
    if state.version != STATE_VERSION {
        return Err(format!(
            "state.json version {} is not supported (this NIRULAUNCHER reads version {STATE_VERSION})",
            state.version
        ));
    }
    state.monitors.retain(|m| m.width > 0 && m.height > 0);
    state.monitors.sort_by_key(|m| m.index);
    for monitor in &mut state.monitors {
        monitor.tags.retain(|t| t.tag > 0);
        if monitor.current_tag > 0 && !monitor.tags.iter().any(|t| t.tag == monitor.current_tag) {
            monitor.tags.push(Tag {
                tag: monitor.current_tag,
                name: String::new(),
                thumbnail: String::new(),
                windows: Vec::new(),
            });
        }
        monitor.tags.sort_by_key(|t| t.tag);
        monitor.tags.dedup_by_key(|t| t.tag);
    }
    if state.monitors.is_empty() {
        return Err("state.json lists no monitors".into());
    }
    Ok(state)
}

/// A decoded picture, 8-bit RGB.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Image {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) rgb: Vec<u8>,
}

/// Decode a binary PPM (`P6`). Larger sample depths are reduced to 8 bits.
pub(crate) fn parse_ppm(bytes: &[u8]) -> Option<Image> {
    let mut pos = 0usize;
    let mut token = || -> Option<&[u8]> {
        loop {
            match bytes.get(pos)? {
                b'#' => {
                    while *bytes.get(pos)? != b'\n' {
                        pos += 1;
                    }
                }
                c if c.is_ascii_whitespace() => pos += 1,
                _ => break,
            }
        }
        let start = pos;
        while bytes.get(pos).is_some_and(|c| !c.is_ascii_whitespace()) {
            pos += 1;
        }
        Some(&bytes[start..pos])
    };
    if token()? != b"P6" {
        return None;
    }
    let mut number = || -> Option<usize> { std::str::from_utf8(token()?).ok()?.parse().ok() };
    let width = number()?;
    let height = number()?;
    let max = number()?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 || !(1..=65535).contains(&max) {
        return None;
    }
    // Exactly one whitespace byte separates the header from the samples.
    if !bytes.get(pos)?.is_ascii_whitespace() {
        return None;
    }
    pos += 1;
    let wide = max > 255;
    let samples = width * height * 3;
    let data = bytes.get(pos..pos + samples * if wide { 2 } else { 1 })?;
    let rgb = if wide {
        data.chunks_exact(2)
            .map(|s| ((u32::from(s[0]) << 8 | u32::from(s[1])) * 255 / max as u32) as u8)
            .collect()
    } else if max == 255 {
        data.to_vec()
    } else {
        data.iter()
            .map(|&s| (u32::from(s) * 255 / max as u32).min(255) as u8)
            .collect()
    };
    Some(Image { width, height, rgb })
}

/// The largest size with the picture's aspect that fits in the box.
pub(crate) fn fit(width: usize, height: usize, box_w: u16, box_h: u16) -> (u16, u16) {
    if width == 0 || height == 0 || box_w == 0 || box_h == 0 {
        return (0, 0);
    }
    let (bw, bh) = (usize::from(box_w), usize::from(box_h));
    if width * bh > height * bw {
        (box_w, ((height * bw / width).max(1)) as u16)
    } else {
        (((width * bh / height).max(1)) as u16, box_h)
    }
}

/// Scale with a box filter (an average of the source pixels each target
/// pixel covers; nearest neighbour when enlarging). Pixels are 0x00RRGGBB.
pub(crate) fn scale(image: &Image, width: u16, height: u16) -> Vec<u32> {
    let (tw, th) = (usize::from(width), usize::from(height));
    let mut out = Vec::with_capacity(tw * th);
    for y in 0..th {
        let y0 = y * image.height / th;
        let y1 = ((y + 1) * image.height / th).max(y0 + 1).min(image.height);
        for x in 0..tw {
            let x0 = x * image.width / tw;
            let x1 = ((x + 1) * image.width / tw).max(x0 + 1).min(image.width);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                let row = sy * image.width;
                for sx in x0..x1 {
                    let p = (row + sx) * 3;
                    r += u32::from(image.rgb[p]);
                    g += u32::from(image.rgb[p + 1]);
                    b += u32::from(image.rgb[p + 2]);
                    n += 1;
                }
            }
            let n = n.max(1);
            out.push((r / n) << 16 | (g / n) << 8 | (b / n));
        }
    }
    out
}

/// Only a plain file name in the overview directory may be read.
fn thumbnail_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let ok = !name.is_empty()
        && !name.contains('/')
        && name != "."
        && name != ".."
        && name.ends_with(".ppm");
    ok.then(|| dir.join(name))
}

fn load_thumbnail(dir: &Path, name: &str) -> Option<Image> {
    let path = thumbnail_path(dir, name)?;
    let meta = std::fs::metadata(&path).ok()?;
    // 384 × 384 × 3 bytes plus a header; anything far larger is not ours.
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    parse_ppm(&std::fs::read(path).ok()?)
}

/// One card on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Card {
    pub(crate) section: usize,
    pub(crate) tag: usize,
    pub(crate) x: i16,
    pub(crate) y: i16,
    pub(crate) w: u16,
    pub(crate) h: u16,
    /// Thumbnail box inside the card.
    pub(crate) thumb_w: u16,
    pub(crate) thumb_h: u16,
}

impl Card {
    fn contains(&self, x: i16, y: i16) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i16 && y < self.y + self.h as i16
    }

    fn centre(&self) -> (i32, i32) {
        (
            i32::from(self.x) + i32::from(self.w) / 2,
            i32::from(self.y) + i32::from(self.h) / 2,
        )
    }
}

/// Where everything goes, in window coordinates.
#[derive(Debug, PartialEq)]
pub(crate) struct Layout {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) pad: i16,
    /// The top of each section's title line.
    pub(crate) sections: Vec<i16>,
    pub(crate) cards: Vec<Card>,
}

/// Text lines under each thumbnail: the tag and two window lines.
pub(crate) const TEXT_LINES: i16 = 3;

/// The space around a card's thumbnail and text.
fn margin(pad: i16) -> i16 {
    (pad / 2).max(6)
}

/// Lay out the sections: one card size for all, shrunk until the whole
/// overview fits in `max_w` × `max_h`. `sections` holds each monitor's
/// width, height and tag count.
pub(crate) fn layout(sections: &[(u32, u32, usize)], max_w: u16, max_h: u16, line: i16) -> Layout {
    let pad = (line / 2).max(8);
    let cols = sections
        .iter()
        .map(|s| s.2.clamp(1, MAX_COLS))
        .max()
        .unwrap_or(1)
        .max(3);
    let header = line + pad * 2;
    let section_title = line + pad / 2;
    let m = margin(pad);
    // Margin, thumbnail, margin, text lines, margin.
    let text = TEXT_LINES * line + m * 3 + line / 4;
    let aspect = |(w, h, _): (u32, u32, usize), tw: u16| -> u16 {
        // Very tall or very wide monitors still get a usable card.
        let th = (u64::from(tw) * u64::from(h) / u64::from(w.max(1))) as u16;
        th.clamp(tw / 3, tw.saturating_mul(2)).max(1)
    };
    let mut thumb_w = (((i32::from(max_w) - i32::from(pad) * (cols as i32 + 1)) / cols as i32
        - i32::from(m) * 2)
        .clamp(48, i32::from(MAX_THUMB))) as u16;
    loop {
        let total: i32 = i32::from(header)
            + sections
                .iter()
                .map(|&s| {
                    let rows = s.2.max(1).div_ceil(cols) as i32;
                    i32::from(section_title)
                        + rows * (i32::from(aspect(s, thumb_w)) + i32::from(text) + i32::from(pad))
                })
                .sum::<i32>()
            + i32::from(pad);
        if total <= i32::from(max_h) || thumb_w <= 48 {
            break;
        }
        thumb_w = (thumb_w - thumb_w / 12).max(48);
    }
    let card_w = thumb_w + 2 * m as u16;
    let width = ((card_w as i32 + i32::from(pad)) * cols as i32 + i32::from(pad))
        .min(i32::from(max_w))
        .max(200) as u16;
    let mut y = header;
    let mut cards = Vec::new();
    let mut tops = Vec::new();
    for (section, &s) in sections.iter().enumerate() {
        tops.push(y);
        y += section_title;
        let thumb_h = aspect(s, thumb_w);
        let card_h = thumb_h + text as u16;
        for tag in 0..s.2 {
            let (row, col) = (tag / cols, tag % cols);
            cards.push(Card {
                section,
                tag,
                x: pad + (card_w as i16 + pad) * col as i16,
                y: y + (card_h as i16 + pad) * row as i16,
                w: card_w,
                h: card_h,
                thumb_w,
                thumb_h,
            });
        }
        let rows = s.2.max(1).div_ceil(cols) as i16;
        y += rows * (card_h as i16 + pad);
    }
    let height = (y + pad).min(max_h as i16).max(header + line) as u16;
    Layout {
        width,
        height,
        pad,
        sections: tops,
        cards,
    }
}

/// Keyboard movement between cards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Move {
    Left,
    Right,
    Up,
    Down,
    Next,
    Prev,
}

/// The card selected after a move. Left and Right step through the cards in
/// reading order, Tab and Shift+Tab do the same and wrap, Up and Down go to
/// the nearest card in the row above or below, also across monitors.
pub(crate) fn step(cards: &[Card], from: usize, dir: Move) -> usize {
    let n = cards.len();
    if n == 0 {
        return 0;
    }
    let from = from.min(n - 1);
    match dir {
        Move::Left => from.saturating_sub(1),
        Move::Right => (from + 1).min(n - 1),
        Move::Next => (from + 1) % n,
        Move::Prev => (from + n - 1) % n,
        Move::Up | Move::Down => {
            let (cx, cy) = cards[from].centre();
            cards
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    let y = c.centre().1;
                    if dir == Move::Up {
                        y < cy
                    } else {
                        y > cy
                    }
                })
                .min_by_key(|(_, c)| {
                    let (x, y) = c.centre();
                    ((y - cy).abs(), (x - cx).abs())
                })
                .map(|(i, _)| i)
                .unwrap_or(from)
        }
    }
}

/// What a digit key means: tag 1–9, and 0 for tag 10.
pub(crate) fn digit_tag(sym: u32) -> Option<u32> {
    let digit = match sym {
        0x30..=0x39 => sym - 0x30,
        0xffb0..=0xffb9 => sym - 0xffb0,
        _ => return None,
    };
    Some(if digit == 0 { 10 } else { digit })
}

/// `niructl do goto MONITOR TAG [WINDOW]`.
pub(crate) fn goto_argv(monitor: u32, tag: u32, window: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        "niructl".to_string(),
        "do".into(),
        "goto".into(),
        monitor.to_string(),
        tag.to_string(),
    ];
    if let Some(id) = window.filter(|id| valid_window_id(id)) {
        argv.push(id.to_string());
    }
    argv
}

fn valid_window_id(id: &str) -> bool {
    match id.strip_prefix("0x").or_else(|| id.strip_prefix("0X")) {
        Some(hex) => {
            !hex.is_empty() && hex.len() <= 8 && hex.chars().all(|c| c.is_ascii_hexdigit())
        }
        None => !id.is_empty() && id.len() <= 10 && id.chars().all(|c| c.is_ascii_digit()),
    }
}

/// The window NIRUWM should focus on that tag: the one it had focused.
fn focused_window(tag: &Tag) -> Option<&str> {
    tag.windows
        .iter()
        .find(|w| w.focused)
        .map(|w| w.id.as_str())
}

/// The two window lines of a card.
pub(crate) fn window_lines(windows: &[Win]) -> [String; 2] {
    match windows {
        [] => ["No windows".into(), String::new()],
        [one] => [one.label(), String::new()],
        [one, two] => [one.label(), two.label()],
        [one, rest @ ..] => [one.label(), format!("+{} more", rest.len())],
    }
}

/// The age of `state.json` when it is old enough to mention.
pub(crate) fn stale_note(age_secs: u64) -> Option<String> {
    (age_secs >= STALE_SECS).then(|| {
        let minutes = age_secs / 60;
        if minutes >= 120 {
            format!("saved {} h ago", minutes / 60)
        } else {
            format!("saved {minutes} min ago")
        }
    })
}

/// Read the state, or the short message to show instead.
fn load() -> std::result::Result<(State, PathBuf, Option<String>), String> {
    let dir = state_dir().ok_or("XDG_RUNTIME_DIR is not set, so NIRUWM keeps no overview")?;
    let path = dir.join("state.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("no workspace state from NIRUWM ({}: {e})", path.display()))?;
    let state = parse_state(&text)?;
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok((state, dir, stale_note(age)))
}

struct Overlay<'a> {
    c: &'a RustConnection,
    win: Window,
    gc: Gcontext,
    selection: Atom,
}

impl<'a> Overlay<'a> {
    /// A managed dialog like the other views: centred on `area`, keyboard
    /// grabbed, and the lock-screen handoff selection owned.
    fn open(
        c: &'a RustConnection,
        root: Window,
        cfg: &Config,
        area: (i16, i16, u16, u16),
        width: u16,
        height: u16,
    ) -> Result<Self> {
        let selection = intern(c, "_NIRULANCHER_SELECTION")?;
        let activate = intern(c, "_NIRULANCHER_ACTIVATE")?;
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
        let (ax, ay, aw, ah) = area;
        let x = ax + (aw as i16 - width as i16).max(0) / 2;
        let y = ay + (ah as i16 - height as i16).max(0) / 2;
        let win = c.generate_id()?;
        c.create_window(
            0,
            win,
            root,
            x,
            y,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(cfg.palette.background)
                .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
        )?;
        let gc = c.generate_id()?;
        c.create_gc(gc, win, &CreateGCAux::new())?;
        set_utf8(c, win, intern(c, "_NET_WM_NAME")?, "NIRULANCHER")?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
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
            intern(c, "_NET_WM_WINDOW_TYPE")?,
            AtomEnum::ATOM,
            &[intern(c, "_NET_WM_WINDOW_TYPE_DIALOG")?],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            intern(c, "_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        // Ask the window manager to keep the size and place it here.
        let hints = [
            1 | 4 | 8 | 16 | 32, // USPosition, PPosition, PSize, PMinSize, PMaxSize
            x as i32 as u32,
            y as i32 as u32,
            u32::from(width),
            u32::from(height),
            u32::from(width),
            u32::from(height),
            u32::from(width),
            u32::from(height),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        c.change_property32(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            &hints,
        )?;
        c.set_selection_owner(win, selection, x11rb::CURRENT_TIME)?;
        c.map_window(win)?;
        wait_until_viewable(c, win)?;
        // The Super key that opened the overview may still be held by the
        // window manager's passive grab for a moment.
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let status = c
                .grab_keyboard(
                    false,
                    win,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )?
                .reply()
                .map(|r| r.status)
                .unwrap_or(GrabStatus::ALREADY_GRABBED);
            if status == GrabStatus::SUCCESS || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
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
        Ok(Self {
            c,
            win,
            gc,
            selection,
        })
    }

    fn close(&self) {
        let _ = self.c.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.c.ungrab_pointer(x11rb::CURRENT_TIME);
        let _ = self
            .c
            .set_selection_owner(x11rb::NONE, self.selection, x11rb::CURRENT_TIME);
        let _ = self.c.destroy_window(self.win);
        let _ = self.c.flush();
    }
}

/// What the event loop ended with.
enum Outcome {
    Closed,
    Goto(Vec<String>),
}

pub(crate) fn run(cfg: &Config) -> Result<()> {
    let (c, screen) = x11rb::connect(None)?;
    let root = c.setup().roots[screen].root;
    match load() {
        Ok((state, dir, note)) => run_overview(&c, root, cfg, &state, &dir, note),
        Err(message) => {
            eprintln!("nirulauncher: {message}");
            run_message(&c, root, cfg)
        }
    }
}

fn baseline(top: i16, line: i16) -> i16 {
    top + line - line / 5
}

/// The short window shown when there is no overview to show.
fn run_message(c: &RustConnection, root: Window, cfg: &Config) -> Result<()> {
    let lines = [
        "No workspace overview yet",
        "NIRUWM writes it when you open the overview (Super+E).",
        "Press any key to close.",
    ];
    let (advance, line) = Renderer::measure(cfg.font_path.as_deref(), cfg.font_size)?;
    let pad = (line / 2).max(8) * 2;
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as i16;
    let width = (advance * longest + pad * 2) as u16;
    let height = (line * lines.len() as i16 + line / 2 + pad * 2) as u16;
    let (px, py) = point(c, root);
    let m = monitor_at(c, root, px, py);
    let overlay = Overlay::open(c, root, cfg, (m.x, m.y, m.w, m.h), width, height)?;
    let mut renderer = Renderer::new(
        c,
        root,
        cfg.font_path.as_deref(),
        cfg.font_size,
        width,
        height,
    )?;
    let pal = &cfg.palette;
    let lock = intern(c, "_NIRULANCHER_LOCK")?;
    let draw = |renderer: &mut Renderer| -> Result<()> {
        renderer.clear(c, overlay.gc, pal.background)?;
        frame(renderer, c, overlay.gc, width, height, pal.border)?;
        for (i, text) in lines.iter().enumerate() {
            let top = pad + line * i as i16 + if i > 0 { line / 2 } else { 0 };
            let colour = if i == 0 { pal.foreground } else { pal.muted };
            renderer.line(
                c,
                overlay.gc,
                pad,
                baseline(top, line),
                text,
                colour,
                pal.background,
            )?;
        }
        renderer.present(c, overlay.gc, overlay.win, width)?;
        c.flush()?;
        Ok(())
    };
    draw(&mut renderer)?;
    loop {
        match c.wait_for_event()? {
            Event::Expose(e) if e.count == 0 => draw(&mut renderer)?,
            Event::KeyPress(_) | Event::ButtonPress(_) => break,
            Event::ClientMessage(e) if e.type_ == lock => break,
            Event::SelectionClear(_) => break,
            _ => {}
        }
    }
    overlay.close();
    Ok(())
}

fn frame(
    renderer: &Renderer,
    c: &RustConnection,
    gc: Gcontext,
    width: u16,
    height: u16,
    colour: u32,
) -> Result<()> {
    rect_outline(renderer, c, gc, 0, 0, width, height, 1, colour)
}

#[allow(clippy::too_many_arguments)]
fn rect_outline(
    renderer: &Renderer,
    c: &RustConnection,
    gc: Gcontext,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    t: u16,
    colour: u32,
) -> Result<()> {
    let r = |x: i16, y: i16, width: u16, height: u16| Rectangle {
        x,
        y,
        width,
        height,
    };
    renderer.fill(c, gc, r(x, y, w, t), colour)?;
    renderer.fill(c, gc, r(x, y + h as i16 - t as i16, w, t), colour)?;
    renderer.fill(c, gc, r(x, y, t, h), colour)?;
    renderer.fill(c, gc, r(x + w as i16 - t as i16, y, t, h), colour)?;
    Ok(())
}

/// A scaled thumbnail and where it sits in its box.
struct Thumb {
    x: i16,
    y: i16,
    w: u16,
    pixels: Vec<u32>,
}

fn run_overview(
    c: &RustConnection,
    root: Window,
    cfg: &Config,
    state: &State,
    dir: &Path,
    note: Option<String>,
) -> Result<()> {
    let (advance, line) = Renderer::measure(cfg.font_path.as_deref(), cfg.font_size)?;
    // On the focused monitor; NIRUWM's geometry first, the pointer's monitor
    // when the state has none marked.
    let focused = state.monitors.iter().position(|m| m.focused).unwrap_or(0);
    let area = {
        let m = &state.monitors[focused];
        if state.monitors.iter().any(|m| m.focused) {
            (
                m.x.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
                m.y.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
                m.width.min(u16::MAX.into()) as u16,
                m.height.min(u16::MAX.into()) as u16,
            )
        } else {
            let (px, py) = point(c, root);
            let r = monitor_at(c, root, px, py);
            (r.x, r.y, r.w, r.h)
        }
    };
    let max_w = ((u32::from(area.2) * 9 / 10).min(1800)) as u16;
    let max_h = ((u32::from(area.3) * 9 / 10).max(200)) as u16;
    let sections: Vec<(u32, u32, usize)> = state
        .monitors
        .iter()
        .map(|m| (m.width, m.height, m.tags.len()))
        .collect();
    let lay = layout(&sections, max_w, max_h, line);
    let (width, height, pad) = (lay.width, lay.height, lay.pad);
    let thumbs: Vec<Option<Thumb>> = lay
        .cards
        .iter()
        .map(|card| {
            let tag = &state.monitors[card.section].tags[card.tag];
            let image = load_thumbnail(dir, &tag.thumbnail)?;
            let (tw, th) = fit(image.width, image.height, card.thumb_w, card.thumb_h);
            Some(Thumb {
                x: card.x + margin(pad) + (card.thumb_w as i16 - tw as i16) / 2,
                y: card.y + margin(pad) + (card.thumb_h as i16 - th as i16) / 2,
                w: tw,
                pixels: scale(&image, tw, th),
            })
        })
        .collect();
    let mut selected = lay
        .cards
        .iter()
        .position(|card| {
            let m = &state.monitors[card.section];
            card.section == focused && m.tags[card.tag].tag == m.current_tag
        })
        .unwrap_or(0);
    let overlay = Overlay::open(c, root, cfg, area, width, height)?;
    let mut renderer = Renderer::new(
        c,
        root,
        cfg.font_path.as_deref(),
        cfg.font_size,
        width,
        height,
    )?;
    let gc = overlay.gc;
    let pal = cfg.palette;
    let lock = intern(c, "_NIRULANCHER_LOCK")?;
    let activate = intern(c, "_NIRULANCHER_ACTIVATE")?;
    let chars = |px: i32| -> i16 { (px / i32::from(advance.max(1))).max(1) as i16 };
    let draw = |renderer: &mut Renderer, selected: usize| -> Result<()> {
        renderer.clear(c, gc, pal.background)?;
        frame(renderer, c, gc, width, height, pal.border)?;
        let title = "Workspaces";
        renderer.line(
            c,
            gc,
            pad,
            baseline(pad, line),
            title,
            pal.foreground,
            pal.background,
        )?;
        let mut hint = String::from("1-9 tag · arrows move · Enter go · Esc close");
        if let Some(note) = &note {
            hint = format!("{note} · {hint}");
        }
        let room = chars(i32::from(width) - i32::from(pad) * 3) - title.chars().count() as i16;
        if room > 8 {
            let hint = crate::ui::truncate(&hint, room);
            let x = width as i16 - pad - advance * hint.chars().count() as i16;
            renderer.line(
                c,
                gc,
                x,
                baseline(pad, line),
                &hint,
                pal.muted,
                pal.background,
            )?;
        }
        for (section, &top) in lay.sections.iter().enumerate() {
            let m = &state.monitors[section];
            let mut text = format!("Monitor {}", m.index + 1);
            if !m.name.is_empty() {
                text.push_str(" · ");
                text.push_str(&m.name);
            }
            if section == focused && state.monitors.len() > 1 {
                text.push_str(" · focused");
            }
            let text = crate::ui::truncate(&text, chars(i32::from(width) - i32::from(pad) * 2));
            let colour = if section == focused {
                pal.foreground
            } else {
                pal.muted
            };
            renderer.line(
                c,
                gc,
                pad,
                baseline(top, line),
                &text,
                colour,
                pal.background,
            )?;
        }
        for (i, card) in lay.cards.iter().enumerate() {
            let mon = &state.monitors[card.section];
            let tag = &mon.tags[card.tag];
            let bg = if i == selected {
                pal.selected
            } else {
                pal.surface
            };
            renderer.fill(
                c,
                gc,
                Rectangle {
                    x: card.x,
                    y: card.y,
                    width: card.w,
                    height: card.h,
                },
                bg,
            )?;
            let m = margin(pad);
            let (bx, by) = (card.x + m, card.y + m);
            renderer.fill(
                c,
                gc,
                Rectangle {
                    x: bx,
                    y: by,
                    width: card.thumb_w,
                    height: card.thumb_h,
                },
                pal.background,
            )?;
            match &thumbs[i] {
                Some(t) => renderer.pixels(c, gc, t.x, t.y, t.w, &t.pixels)?,
                None => {
                    let text = if tag.windows.is_empty() {
                        "empty"
                    } else {
                        "no preview"
                    };
                    let text = crate::ui::truncate(text, chars(i32::from(card.thumb_w)));
                    let x = bx + (card.thumb_w as i16 - advance * text.chars().count() as i16) / 2;
                    let top = by + (card.thumb_h as i16 - line) / 2;
                    renderer.line(
                        c,
                        gc,
                        x,
                        baseline(top, line),
                        &text,
                        pal.muted,
                        pal.background,
                    )?;
                }
            }
            let text_x = bx;
            let max = chars(i32::from(card.w) - i32::from(m) * 2);
            let mut top = by + card.thumb_h as i16 + m + line / 8;
            let mut label = tag.tag.to_string();
            if !tag.name.is_empty() && tag.name != label {
                label.push_str(" · ");
                label.push_str(&tag.name);
            }
            if tag.tag == mon.current_tag {
                label.push_str(" · shown");
            }
            let label_colour = pal.foreground;
            renderer.line(
                c,
                gc,
                text_x,
                baseline(top, line),
                &crate::ui::truncate(&label, max),
                label_colour,
                bg,
            )?;
            for (n, text) in window_lines(&tag.windows).iter().enumerate() {
                top += line;
                if text.is_empty() {
                    continue;
                }
                let colour = if n == 0 && !tag.windows.is_empty() {
                    pal.foreground
                } else {
                    pal.muted
                };
                renderer.line(
                    c,
                    gc,
                    text_x,
                    baseline(top, line),
                    &crate::ui::truncate(text, max),
                    colour,
                    bg,
                )?;
            }
            if i == selected {
                rect_outline(
                    renderer, c, gc, card.x, card.y, card.w, card.h, 3, pal.accent,
                )?;
            }
        }
        renderer.present(c, gc, overlay.win, width)?;
        // The event wait does not flush; without this the frame never shows.
        c.flush()?;
        Ok(())
    };
    let pick = |i: usize| -> Option<Outcome> {
        let card = lay.cards.get(i)?;
        let m = &state.monitors[card.section];
        let tag = &m.tags[card.tag];
        Some(Outcome::Goto(goto_argv(
            m.index,
            tag.tag,
            focused_window(tag),
        )))
    };
    draw(&mut renderer, selected)?;
    let mut keymap = Keymap::load(c);
    let outcome = loop {
        match c.wait_for_event()? {
            Event::Expose(e) if e.count == 0 => draw(&mut renderer, selected)?,
            Event::MappingNotify(_) => keymap = Keymap::load(c),
            Event::KeyPress(key) => {
                let sym = keymap.symbol(key.detail, key.state);
                let shift = key.state.contains(KeyButMask::SHIFT);
                let dir = match sym {
                    0xff1b => break Outcome::Closed,
                    0xff0d | 0xff8d => match pick(selected) {
                        Some(o) => break o,
                        None => break Outcome::Closed,
                    },
                    0xff51 => Some(Move::Left),
                    0xff53 => Some(Move::Right),
                    0xff52 => Some(Move::Up),
                    0xff54 => Some(Move::Down),
                    0xff09 if shift => Some(Move::Prev),
                    0xff09 => Some(Move::Next),
                    0xfe20 => Some(Move::Prev),
                    0xff50 => {
                        selected = 0;
                        None
                    }
                    0xff57 => {
                        selected = lay.cards.len().saturating_sub(1);
                        None
                    }
                    _ => {
                        if let Some(tag) = digit_tag(sym) {
                            let m = &state.monitors[focused];
                            let window = m
                                .tags
                                .iter()
                                .find(|t| t.tag == tag)
                                .and_then(focused_window);
                            break Outcome::Goto(goto_argv(m.index, tag, window));
                        }
                        continue;
                    }
                };
                if let Some(dir) = dir {
                    selected = step(&lay.cards, selected, dir);
                }
                draw(&mut renderer, selected)?;
            }
            Event::ButtonPress(e) => {
                if e.event != overlay.win {
                    break Outcome::Closed;
                }
                if !matches!(e.detail, 1..=3) {
                    continue;
                }
                if let Some(i) = lay
                    .cards
                    .iter()
                    .position(|card| card.contains(e.event_x, e.event_y))
                {
                    if let Some(o) = pick(i) {
                        break o;
                    }
                }
            }
            Event::ClientMessage(e) if e.type_ == lock => break Outcome::Closed,
            Event::ClientMessage(e) if e.type_ == activate => {
                let _ = c.configure_window(
                    overlay.win,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                );
                c.flush()?;
            }
            Event::SelectionClear(_) => break Outcome::Closed,
            Event::Error(e) => eprintln!("nirulauncher: X11 error: {e:?}"),
            _ => {}
        }
    };
    overlay.close();
    if let Outcome::Goto(argv) = outcome {
        run_niructl(&argv)?;
    }
    Ok(())
}

/// Run `niructl do goto …`, waiting at most two seconds for its answer.
fn run_niructl(argv: &[String]) -> Result<()> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run niructl: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read as _;
                let _ = stderr.read_to_string(&mut err);
            }
            let err = err.trim();
            return Err(if err.is_empty() {
                format!("{} failed ({status})", argv.join(" ")).into()
            } else {
                format!("{}: {err}", argv.join(" ")).into()
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("niructl did not answer in two seconds".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = r#"{"version":1,
     "monitors":[{"index":1,"name":"HDMI-1","x":2560,"y":0,"width":1920,"height":1080,
                  "focused":false,"current_tag":2,"tags":[]},
                 {"index":0,"name":"DP-4","x":0,"y":0,"width":2560,"height":1440,
                  "focused":true,"current_tag":1,
                  "tags":[{"tag":3,"name":"","thumbnail":"mon0-tag3.ppm","captured":1,
                           "windows":[{"id":"0x1a00007","class":"kitty","title":"~","focused":true},
                                      {"id":"0x1a00009","class":"kitty","title":"","focused":false},
                                      {"id":"0x1c00001","class":"Gimp","title":"GIMP","focused":false}]},
                          {"tag":1,"name":"web","thumbnail":"mon0-tag1.ppm","captured":1,"windows":[]}]}]}"#;

    #[test]
    fn the_state_is_ordered_and_keeps_the_current_tag() {
        let state = parse_state(STATE).unwrap();
        assert_eq!(state.monitors[0].index, 0);
        let tags: Vec<u32> = state.monitors[0].tags.iter().map(|t| t.tag).collect();
        assert_eq!(tags, [1, 3]);
        // The second monitor listed no tags; its current tag is added.
        assert_eq!(state.monitors[1].tags.len(), 1);
        assert_eq!(state.monitors[1].tags[0].tag, 2);
        assert!(state.monitors[1].tags[0].thumbnail.is_empty());
    }

    #[test]
    fn unknown_or_broken_state_is_refused() {
        assert!(parse_state(r#"{"version":2,"monitors":[]}"#)
            .unwrap_err()
            .contains("version 2"));
        assert!(parse_state("{").is_err());
        assert!(parse_state(r#"{"version":1,"monitors":[]}"#).is_err());
        // Extra fields from a newer NIRUWM are fine.
        let extra = r#"{"version":1,"future":true,"monitors":[{"index":0,"width":10,"height":10,"current_tag":1,"x":0,"y":0,"new":1}]}"#;
        assert_eq!(parse_state(extra).unwrap().monitors[0].tags[0].tag, 1);
    }

    #[test]
    fn ppm_headers_with_comments_and_depths() {
        let mut bytes = b"P6\n# niruwm\n2 1\n255\n".to_vec();
        bytes.extend([255, 0, 0, 0, 0, 255]);
        let image = parse_ppm(&bytes).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgb, [255, 0, 0, 0, 0, 255]);
        let mut wide = b"P6 1 1 65535\n".to_vec();
        wide.extend([0xff, 0xff, 0x80, 0x00, 0x00, 0x00]);
        assert_eq!(parse_ppm(&wide).unwrap().rgb, [255, 127, 0]);
        let mut low = b"P6 1 1 15 ".to_vec();
        low.extend([15, 0, 7]);
        assert_eq!(parse_ppm(&low).unwrap().rgb, [255, 0, 119]);
        // Short data, another format, absurd sizes.
        assert!(parse_ppm(b"P6 2 2 255\n\0\0\0").is_none());
        assert!(parse_ppm(b"P3 1 1 255\n0 0 0").is_none());
        assert!(parse_ppm(b"P6 99999 1 255\n").is_none());
        assert!(parse_ppm(b"").is_none());
    }

    #[test]
    fn box_scaling_averages_and_keeps_the_aspect() {
        let image = Image {
            width: 4,
            height: 2,
            rgb: [
                [255, 255, 255],
                [255, 255, 255],
                [0, 0, 0],
                [0, 0, 0],
                [255, 255, 255],
                [255, 255, 255],
                [0, 0, 0],
                [0, 0, 0],
            ]
            .concat(),
        };
        assert_eq!(scale(&image, 2, 1), [0xffffff, 0x000000]);
        assert_eq!(scale(&image, 1, 1), [0x7f7f7f]);
        assert_eq!(scale(&image, 8, 4).len(), 32);
        assert_eq!(fit(384, 216, 200, 200), (200, 112));
        assert_eq!(fit(216, 384, 200, 200), (112, 200));
        assert_eq!(fit(0, 10, 10, 10), (0, 0));
    }

    #[test]
    fn thumbnails_are_plain_names_in_the_directory() {
        let dir = Path::new("/run/user/1/niruwm/overview");
        assert!(thumbnail_path(dir, "mon0-tag1.ppm").is_some());
        assert!(thumbnail_path(dir, "../secret.ppm").is_none());
        assert!(thumbnail_path(dir, "/etc/x.ppm").is_none());
        assert!(thumbnail_path(dir, "").is_none());
        assert!(thumbnail_path(dir, "state.json").is_none());
    }

    #[test]
    fn the_layout_fits_and_cards_do_not_overlap() {
        for (sections, w, h) in [
            (vec![(2560, 1440, 2), (1920, 1080, 9)], 1800u16, 1200u16),
            (vec![(1920, 1080, 10)], 1200, 700),
            (vec![(1366, 768, 1)], 1200, 640),
            (vec![(1080, 1920, 4), (3840, 1080, 3)], 1600, 900),
        ] {
            let lay = layout(&sections, w, h, 20);
            assert!(lay.width <= w && lay.height <= h, "{sections:?}");
            let total: usize = sections.iter().map(|s| s.2).sum();
            assert_eq!(lay.cards.len(), total);
            for (i, a) in lay.cards.iter().enumerate() {
                assert!(a.x >= 0 && a.x as i32 + a.w as i32 <= lay.width as i32);
                assert!(a.thumb_w <= a.w && a.thumb_h < a.h);
                for b in &lay.cards[i + 1..] {
                    let apart = a.x + a.w as i16 <= b.x
                        || b.x + b.w as i16 <= a.x
                        || a.y + a.h as i16 <= b.y
                        || b.y + b.h as i16 <= a.y;
                    assert!(apart, "{a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn keys_move_through_the_cards() {
        // Two sections: five cards in one row and a second row of two, then one.
        let lay = layout(&[(1920, 1080, 7), (1920, 1080, 1)], 1800, 1400, 20);
        let cards = &lay.cards;
        assert_eq!(step(cards, 0, Move::Left), 0);
        assert_eq!(step(cards, 0, Move::Right), 1);
        assert_eq!(step(cards, 4, Move::Right), 5);
        assert_eq!(step(cards, 7, Move::Right), 7);
        assert_eq!(step(cards, 7, Move::Next), 0);
        assert_eq!(step(cards, 0, Move::Prev), 7);
        assert_eq!(step(cards, 1, Move::Down), 6);
        assert_eq!(step(cards, 6, Move::Up), 1);
        assert_eq!(step(cards, 6, Move::Down), 7);
        assert_eq!(step(cards, 7, Move::Up), 5);
        assert_eq!(step(cards, 7, Move::Down), 7);
        assert_eq!(step(&[], 3, Move::Down), 0);
    }

    #[test]
    fn digits_name_tags_and_goto_takes_a_window() {
        assert_eq!(digit_tag(0x31), Some(1));
        assert_eq!(digit_tag(0x30), Some(10));
        assert_eq!(digit_tag(0xffb9), Some(9));
        assert_eq!(digit_tag(0x61), None);
        assert_eq!(goto_argv(1, 3, None), ["niructl", "do", "goto", "1", "3"]);
        assert_eq!(goto_argv(0, 2, Some("0x1a00007"))[5], "0x1a00007");
        assert_eq!(goto_argv(0, 2, Some("27262983"))[5], "27262983");
        assert_eq!(goto_argv(0, 2, Some("--evil")).len(), 5);
        assert_eq!(goto_argv(0, 2, Some("0x")).len(), 5);
        let state = parse_state(STATE).unwrap();
        assert_eq!(
            focused_window(&state.monitors[0].tags[1]),
            Some("0x1a00007")
        );
        assert_eq!(focused_window(&state.monitors[0].tags[0]), None);
    }

    #[test]
    fn cards_list_two_window_lines() {
        let state = parse_state(STATE).unwrap();
        assert_eq!(
            window_lines(&state.monitors[0].tags[1].windows),
            ["~".to_string(), "+2 more".to_string()]
        );
        assert_eq!(window_lines(&[])[0], "No windows");
        let two = &state.monitors[0].tags[1].windows[..2];
        // An empty title falls back to the class.
        assert_eq!(window_lines(two), ["~".to_string(), "kitty".to_string()]);
        assert_eq!(stale_note(30), None);
        assert_eq!(stale_note(900).unwrap(), "saved 15 min ago");
        assert_eq!(stale_note(3 * 3600).unwrap(), "saved 3 h ago");
    }
}
