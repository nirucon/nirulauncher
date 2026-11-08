//! Paste a short text into the window the launcher was opened over, then
//! give the clipboard back.
//!
//! Adapted from NIRUDICTATE's insert worker (same author, MIT). Every target
//! the previous clipboard offers is read first (so images and rich text
//! survive), the text is offered as the new clipboard, Ctrl+V (Ctrl+Shift+V
//! in terminals, Shift+Insert with PRIMARY in xterm-like terminals) is sent
//! with XTest, and once the focused program has fetched the text the old
//! content is offered again. This process keeps offering it, with no window,
//! until another program takes the clipboard, the way `xclip` does.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::NONE;

/// WM_CLASS names that paste with Ctrl+Shift+V.
const TERMINALS: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "wezterm",
    "org.wezfurlong.wezterm",
    "st",
    "st-256color",
    "gnome-terminal",
    "gnome-terminal-server",
    "konsole",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "lxterminal",
    "mate-terminal",
    "qterminal",
    "sakura",
    "terminology",
    "ghostty",
    "com.mitchellh.ghostty",
    "contour",
    "rio",
    "termite",
];

/// WM_CLASS names whose paste key is Shift+Insert (PRIMARY selection).
const PRIMARY_TERMINALS: &[&str] = &[
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "urxvt-unicode",
    "rxvt-unicode",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteKey {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}

pub fn paste_key(classes: &[String]) -> PasteKey {
    let lower: Vec<String> = classes.iter().map(|c| c.to_lowercase()).collect();
    if lower
        .iter()
        .any(|c| PRIMARY_TERMINALS.contains(&c.as_str()))
    {
        return PasteKey::ShiftInsert;
    }
    if lower.iter().any(|c| TERMINALS.contains(&c.as_str())) {
        return PasteKey::CtrlShiftV;
    }
    PasteKey::CtrlV
}

/// One selection's saved content: (target, type, format, bytes).
#[derive(Clone, Debug, Default)]
struct Saved {
    items: Vec<(Atom, Atom, u8, Vec<u8>)>,
}

enum Offer {
    Text(String),
    Saved(Saved),
}

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    multiple: Atom,
    utf8: Atom,
    text: Atom,
    text_plain: Atom,
    text_plain_utf8: Atom,
    incr: Atom,
    prop: Atom,
    active: Atom,
    save_targets: Atom,
}

fn intern(conn: &RustConnection, name: &str) -> Result<Atom, String> {
    conn.intern_atom(false, name.as_bytes())
        .map_err(|e| e.to_string())?
        .reply()
        .map(|r| r.atom)
        .map_err(|e| e.to_string())
}

pub struct Paster {
    conn: RustConnection,
    root: Window,
    win: Window,
    atoms: Atoms,
    offers: HashMap<Atom, Offer>,
    /// The focused client fetched the text.
    served: bool,
    focus_base: Option<u32>,
    resource_mask: u32,
    pasting: Option<Atom>,
}

impl Paster {
    pub fn new() -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("X11: {e}"))?;
        let root = conn.setup().roots[screen].root;
        let resource_mask = conn.setup().resource_id_mask;
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            0,
            win,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
        let atoms = Atoms {
            clipboard: intern(&conn, "CLIPBOARD")?,
            targets: intern(&conn, "TARGETS")?,
            timestamp: intern(&conn, "TIMESTAMP")?,
            multiple: intern(&conn, "MULTIPLE")?,
            utf8: intern(&conn, "UTF8_STRING")?,
            text: intern(&conn, "TEXT")?,
            text_plain: intern(&conn, "text/plain")?,
            text_plain_utf8: intern(&conn, "text/plain;charset=utf-8")?,
            incr: intern(&conn, "INCR")?,
            prop: intern(&conn, "_NIRULAUNCHER_SEL")?,
            active: intern(&conn, "_NET_ACTIVE_WINDOW")?,
            save_targets: intern(&conn, "SAVE_TARGETS")?,
        };
        conn.flush().map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            root,
            win,
            atoms,
            offers: HashMap::new(),
            served: false,
            focus_base: None,
            resource_mask,
            pasting: None,
        })
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::SelectionRequest(e) => self.serve(e),
            Event::SelectionClear(e) => {
                self.offers.remove(&e.selection);
            }
            _ => {}
        }
    }

    fn text_targets(&self) -> [Atom; 5] {
        [
            self.atoms.utf8,
            AtomEnum::STRING.into(),
            self.atoms.text,
            self.atoms.text_plain,
            self.atoms.text_plain_utf8,
        ]
    }

    fn serve(&mut self, e: SelectionRequestEvent) {
        let property = if e.property == NONE {
            e.target
        } else {
            e.property
        };
        let max = self.conn.maximum_request_bytes().saturating_sub(1024);
        let mut ok = false;
        if let Some(offer) = self.offers.get(&e.selection) {
            if e.target == self.atoms.targets {
                let mut list = vec![self.atoms.targets, self.atoms.timestamp];
                match offer {
                    Offer::Text(_) => list.extend(self.text_targets()),
                    Offer::Saved(s) => list.extend(s.items.iter().map(|i| i.0)),
                }
                ok = self
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::ATOM,
                        &list,
                    )
                    .is_ok();
            } else if e.target == self.atoms.timestamp {
                ok = self
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::INTEGER,
                        &[0],
                    )
                    .is_ok();
            } else if e.target != self.atoms.multiple {
                match offer {
                    Offer::Text(text) if self.text_targets().contains(&e.target) => {
                        let (ty, data): (Atom, Vec<u8>) = if e.target == u32::from(AtomEnum::STRING)
                        {
                            (AtomEnum::STRING.into(), latin1(text))
                        } else {
                            (self.atoms.utf8, text.as_bytes().to_vec())
                        };
                        if data.len() <= max {
                            ok = self
                                .conn
                                .change_property8(
                                    PropMode::REPLACE,
                                    e.requestor,
                                    property,
                                    ty,
                                    &data,
                                )
                                .is_ok();
                        }
                        if ok
                            && Some(e.selection) == self.pasting
                            && self.focus_base == Some(e.requestor & !self.resource_mask)
                        {
                            self.served = true;
                        }
                    }
                    Offer::Saved(s) => {
                        if let Some((_, ty, format, data)) =
                            s.items.iter().find(|i| i.0 == e.target)
                        {
                            if data.len() <= max {
                                let unit = (*format as usize / 8).max(1);
                                ok = self
                                    .conn
                                    .change_property(
                                        PropMode::REPLACE,
                                        e.requestor,
                                        property,
                                        *ty,
                                        *format,
                                        (data.len() / unit) as u32,
                                        data,
                                    )
                                    .is_ok();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if ok { property } else { NONE },
        };
        let _ = self
            .conn
            .send_event(false, e.requestor, EventMask::NO_EVENT, notify);
        let _ = self.conn.flush();
    }

    fn wait_event<T>(
        &mut self,
        timeout: Duration,
        mut pred: impl FnMut(&Event) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Ok(Some(event)) = self.conn.poll_for_event() {
                if let Some(v) = pred(&event) {
                    return Some(v);
                }
                self.handle(event);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let _ = self.conn.flush();
            std::thread::sleep(left.min(Duration::from_millis(5)));
        }
    }

    fn fetch(&mut self, selection: Atom, target: Atom) -> Option<(Atom, u8, Vec<u8>)> {
        let prop = self.atoms.prop;
        self.conn
            .convert_selection(self.win, selection, target, prop, x11rb::CURRENT_TIME)
            .ok()?;
        self.conn.flush().ok()?;
        let win = self.win;
        let got = self.wait_event(Duration::from_millis(400), |e| match e {
            Event::SelectionNotify(n) if n.requestor == win && n.selection == selection => {
                Some(n.property != NONE)
            }
            _ => None,
        })?;
        if !got {
            return None;
        }
        let reply = self
            .conn
            .get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4)
            .ok()?
            .reply()
            .ok()?;
        if reply.type_ != self.atoms.incr {
            return Some((reply.type_, reply.format, reply.value));
        }
        let mut data = Vec::new();
        let (mut ty, mut format) = (NONE, 8);
        loop {
            let new_value = self.wait_event(Duration::from_millis(1000), |e| match e {
                Event::PropertyNotify(p)
                    if p.window == win
                        && p.atom == prop
                        && p.state == x11rb::protocol::xproto::Property::NEW_VALUE =>
                {
                    Some(())
                }
                _ => None,
            });
            new_value?;
            let chunk = self
                .conn
                .get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4)
                .ok()?
                .reply()
                .ok()?;
            if chunk.value.is_empty() {
                break;
            }
            ty = chunk.type_;
            format = chunk.format;
            data.extend_from_slice(&chunk.value);
            if data.len() > 64 << 20 {
                return None;
            }
        }
        Some((ty, format, data))
    }

    fn save(&mut self, selection: Atom) -> Option<Saved> {
        let owner = self
            .conn
            .get_selection_owner(selection)
            .ok()?
            .reply()
            .ok()?
            .owner;
        if owner == NONE {
            return None;
        }
        let started = Instant::now();
        let mut saved = Saved::default();
        let targets: Vec<Atom> = match self.fetch(selection, self.atoms.targets) {
            Some((_, 32, bytes)) => bytes
                .chunks_exact(4)
                .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            _ => vec![self.atoms.utf8, AtomEnum::STRING.into()],
        };
        let skip = [
            self.atoms.targets,
            self.atoms.timestamp,
            self.atoms.multiple,
            self.atoms.save_targets,
            NONE,
        ];
        let mut total = 0usize;
        for target in targets {
            if skip.contains(&target) || saved.items.iter().any(|i| i.0 == target) {
                continue;
            }
            if started.elapsed() > Duration::from_millis(1500) || total > 48 << 20 {
                break;
            }
            if let Some((ty, format, data)) = self.fetch(selection, target) {
                if ty == NONE || ty == self.atoms.incr {
                    continue;
                }
                total += data.len();
                saved.items.push((target, ty, format, data));
            }
        }
        (!saved.items.is_empty()).then_some(saved)
    }

    fn own(&mut self, selection: Atom, offer: Offer) -> bool {
        self.offers.insert(selection, offer);
        let _ = self
            .conn
            .set_selection_owner(self.win, selection, x11rb::CURRENT_TIME);
        let _ = self.conn.flush();
        self.conn
            .get_selection_owner(selection)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.owner == self.win)
    }

    fn release(&mut self, selection: Atom) {
        self.offers.remove(&selection);
        let owner = self
            .conn
            .get_selection_owner(selection)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.owner);
        if owner == Some(self.win) {
            let _ = self
                .conn
                .set_selection_owner(NONE, selection, x11rb::CURRENT_TIME);
        }
        let _ = self.conn.flush();
    }

    fn active_window(&self) -> Option<Window> {
        let reply = self
            .conn
            .get_property(false, self.root, self.atoms.active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let w = reply.value32()?.next().filter(|w| *w != NONE);
        w
    }

    fn classes(&self, win: Window) -> Vec<String> {
        self.conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.value
                    .split(|b| *b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Another client holds the keyboard (a lock screen, a menu): synthetic
    /// keys would go there, so nothing is pasted.
    fn keyboard_taken(&self) -> bool {
        use x11rb::protocol::xproto::{GrabMode, GrabStatus};
        let status = self
            .conn
            .grab_keyboard(
                false,
                self.root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.status);
        match status {
            Some(GrabStatus::SUCCESS) => {
                let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
                let _ = self.conn.flush();
                false
            }
            Some(GrabStatus::ALREADY_GRABBED) | Some(GrabStatus::FROZEN) => true,
            _ => false,
        }
    }

    /// Wait until the window manager has given the focus back to a window
    /// other than the closed launcher.
    fn wait_for_focus(&self, launcher: Window) -> Option<Window> {
        let deadline = Instant::now() + Duration::from_millis(600);
        loop {
            if let Some(w) = self.active_window().filter(|w| *w != launcher) {
                if !self.keyboard_taken() {
                    return Some(w);
                }
            }
            if Instant::now() >= deadline {
                return self.active_window().filter(|w| *w != launcher);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Paste `text` into the focused window and give the old clipboard
    /// back. Returns how it was pasted; the caller then calls
    /// [`Paster::linger`].
    pub fn paste(&mut self, text: &str, launcher: Window) -> Result<&'static str, String> {
        let Some(focus) = self.wait_for_focus(launcher) else {
            self.own(self.atoms.clipboard, Offer::Text(text.to_string()));
            return Err("no window to paste into".into());
        };
        if self.keyboard_taken() {
            self.own(self.atoms.clipboard, Offer::Text(text.to_string()));
            return Err("the keyboard is held by another program; nothing was pasted".into());
        }
        self.focus_base = Some(focus & !self.resource_mask);
        let key = paste_key(&self.classes(focus));
        let selection = if key == PasteKey::ShiftInsert {
            AtomEnum::PRIMARY.into()
        } else {
            self.atoms.clipboard
        };
        let keyboard = Keyboard::load(&self.conn)?;
        keyboard.wait_for_modifiers_released(&self.conn, Duration::from_millis(800));
        let previous = self.save(selection);
        self.served = false;
        self.pasting = Some(selection);
        if !self.own(selection, Offer::Text(text.to_string())) {
            self.pasting = None;
            return Err("could not take the clipboard".into());
        }
        match key {
            PasteKey::CtrlV => keyboard.chord(&self.conn, &[0xffe3], 0x76)?,
            PasteKey::CtrlShiftV => keyboard.chord(&self.conn, &[0xffe3, 0xffe1], 0x76)?,
            PasteKey::ShiftInsert => keyboard.chord(&self.conn, &[0xffe1], 0xff63)?,
        }
        // Wait until the focused program fetched the text, then a little
        // longer for the follow-up request some toolkits make.
        let deadline = Instant::now() + Duration::from_millis(1200);
        let mut settled: Option<Instant> = None;
        while Instant::now() < deadline {
            let _ = self.wait_event(Duration::from_millis(20), |_| None::<()>);
            if self.served && settled.is_none() {
                settled = Some(Instant::now());
            }
            if settled.is_some_and(|t| t.elapsed() > Duration::from_millis(250)) {
                break;
            }
        }
        let pasted = self.served;
        self.pasting = None;
        if !pasted {
            // Keep the text on the clipboard, so Ctrl+V by hand still works.
            return Err("the window did not take the paste".into());
        }
        match previous {
            Some(saved) => {
                self.own(selection, Offer::Saved(saved));
            }
            None => self.release(selection),
        }
        Ok(match key {
            PasteKey::CtrlV => "Ctrl+V",
            PasteKey::CtrlShiftV => "Ctrl+Shift+V",
            PasteKey::ShiftInsert => "Shift+Insert",
        })
    }

    /// Keep offering the restored clipboard until another program takes it.
    pub fn linger(mut self) {
        while !self.offers.is_empty() {
            match self.conn.wait_for_event() {
                Ok(event) => self.handle(event),
                Err(_) => return,
            }
        }
    }
}

fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect()
}

/// The keyboard mapping, for XTest key events.
struct Keyboard {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keyboard {
    fn load(conn: &RustConnection) -> Result<Self, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            min,
            per: map.keysyms_per_keycode as usize,
            syms: map.keysyms,
        })
    }

    fn code(&self, sym: u32) -> Option<u8> {
        self.syms
            .chunks(self.per.max(1))
            .position(|chunk| chunk.first() == Some(&sym) || chunk.get(1) == Some(&sym))
            .map(|i| self.min + i as u8)
    }

    fn fake(&self, conn: &RustConnection, press: bool, code: u8) -> Result<(), String> {
        let ty = if press { 2 } else { 3 };
        conn.xtest_fake_input(ty, code, x11rb::CURRENT_TIME, NONE, 0, 0, 0)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn sync(conn: &RustConnection) {
        if let Ok(c) = conn.get_input_focus() {
            let _ = c.reply();
        }
    }

    fn held_modifiers(&self, conn: &RustConnection) -> Vec<u8> {
        let Some(keys) = conn.query_keymap().ok().and_then(|c| c.reply().ok()) else {
            return Vec::new();
        };
        let mods = [
            0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xfe03, 0xffe7, 0xffe8,
        ];
        let mut held = Vec::new();
        for sym in mods {
            for (i, chunk) in self.syms.chunks(self.per.max(1)).enumerate() {
                if chunk.first() == Some(&sym) {
                    let code = self.min as usize + i;
                    if keys.keys[code / 8] & (1 << (code % 8)) != 0 && !held.contains(&(code as u8))
                    {
                        held.push(code as u8);
                    }
                }
            }
        }
        held
    }

    /// Shift may still be down from Shift+Enter, or Super from the shortcut.
    /// Wait for the user to let go; after the timeout release them.
    fn wait_for_modifiers_released(&self, conn: &RustConnection, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let held = self.held_modifiers(conn);
            if held.is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                for code in held {
                    let _ = self.fake(conn, false, code);
                }
                let _ = conn.flush();
                Self::sync(conn);
                return;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn chord(&self, conn: &RustConnection, mods: &[u32], key: u32) -> Result<(), String> {
        let codes: Vec<u8> = mods
            .iter()
            .map(|m| self.code(*m).ok_or("modifier key not on this keyboard"))
            .collect::<Result<_, _>>()?;
        let key = self.code(key).ok_or("paste key not on this keyboard")?;
        for c in &codes {
            self.fake(conn, true, *c)?;
        }
        self.fake(conn, true, key)?;
        self.fake(conn, false, key)?;
        for c in codes.iter().rev() {
            self.fake(conn, false, *c)?;
        }
        conn.flush().map_err(|e| e.to_string())?;
        Self::sync(conn);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_get_their_paste_key() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(paste_key(&c(&["kitty", "kitty"])), PasteKey::CtrlShiftV);
        assert_eq!(paste_key(&c(&["xterm", "XTerm"])), PasteKey::ShiftInsert);
        assert_eq!(paste_key(&c(&["Navigator", "firefox"])), PasteKey::CtrlV);
        assert_eq!(paste_key(&[]), PasteKey::CtrlV);
    }
}
