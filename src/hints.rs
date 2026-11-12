//! Window jump (`--hints`, Ctrl+Alt+W in NIRUWM).
//!
//! Every visible window on every screen gets a short label in its middle.
//! Typing a label focuses that window; Escape, a click or any key that
//! matches no label closes the hints. Labels are letters (home row first) or
//! numbers, set with `[hints] style`. They never share a prefix: with more
//! windows than one character can name, every label gets two characters, so
//! a window is chosen the moment its label is complete.

use crate::config::Config;
use crate::input::Keymap;
use crate::render::Renderer;
use crate::ui::intern;
use crate::Result;
use std::time::{Duration, Instant};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// Labels for `count` windows, all of the same length so none is a prefix of another.
pub(crate) fn labels(count: usize, style: &str) -> Vec<String> {
    let alphabet: Vec<char> = if style == "numbers" {
        "123456789".chars().collect()
    } else {
        LETTERS.chars().collect()
    };
    if count <= alphabet.len() {
        return alphabet.iter().take(count).map(|c| c.to_string()).collect();
    }
    if style == "numbers" {
        // 01 … 99: two digits keep every label the same length.
        return (1..=count.min(99)).map(|n| format!("{n:02}")).collect();
    }
    let mut out = Vec::new();
    'outer: for a in &alphabet {
        for b in &alphabet {
            if out.len() == count {
                break 'outer;
            }
            out.push(format!("{a}{b}"));
        }
    }
    out
}

/// Which labels still match what has been typed, and the one that is complete.
pub(crate) fn narrow(labels: &[String], typed: &str) -> (Vec<usize>, Option<usize>) {
    let matching: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with(typed))
        .map(|(i, _)| i)
        .collect();
    let done = matching.iter().copied().find(|&i| labels[i] == typed);
    (matching, done)
}

struct Target {
    window: Window,
    x: i16,
    y: i16,
}

/// Mapped top-level windows with their centre on the root window.
fn visible_windows(c: &RustConnection, root: Window) -> Result<Vec<Target>> {
    let list = intern(c, "_NET_CLIENT_LIST")?;
    let windows: Vec<Window> = c
        .get_property(false, root, list, AtomEnum::WINDOW, 0, 1024)?
        .reply()?
        .value32()
        .map(|v| v.collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for window in windows {
        let Ok(attrs) = c.get_window_attributes(window)?.reply() else {
            continue;
        };
        if attrs.map_state != MapState::VIEWABLE {
            continue;
        }
        let class = c
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
            .reply()
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .unwrap_or_default();
        if class.contains("NIRULANCHER") || class.contains("NIRULOCK") {
            continue;
        }
        let Ok(geometry) = c.get_geometry(window)?.reply() else {
            continue;
        };
        let Ok(origin) = c.translate_coordinates(window, root, 0, 0)?.reply() else {
            continue;
        };
        if geometry.width < 8 || geometry.height < 8 {
            continue;
        }
        out.push(Target {
            window,
            x: origin.dst_x.saturating_add((geometry.width / 2) as i16),
            y: origin.dst_y.saturating_add((geometry.height / 2) as i16),
        });
    }
    // Left to right, then top to bottom, so the first letters land on the
    // left screen's windows the way people read them.
    out.sort_by_key(|t| (t.x / 200, t.y));
    Ok(out)
}

fn grab_keyboard(c: &RustConnection, root: Window) -> Result<()> {
    // The key that started us may still be held by the window manager's
    // passive grab for a moment; try for a quarter of a second.
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        let status = c
            .grab_keyboard(
                true,
                root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .reply()?
            .status;
        if status == GrabStatus::SUCCESS {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("the keyboard is busy; try the window jump again".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn run(cfg: &Config) -> Result<()> {
    let (c, screen) = x11rb::connect(None)?;
    let root = c.setup().roots[screen].root;
    let targets = visible_windows(&c, root)?;
    if targets.is_empty() {
        return Ok(());
    }
    let labels = labels(targets.len(), &cfg.hints_style);
    let font_size = (cfg.font_size * 1.6).max(18.0);
    let (advance, line_height) = Renderer::measure(cfg.font_path.as_deref(), font_size)?;
    let pad = (line_height / 3).max(6);
    let longest = labels.iter().map(|l| l.chars().count()).max().unwrap_or(1) as i16;
    let width = (advance * longest + pad * 2) as u16;
    let height = (line_height + pad) as u16;
    let mut renderer = Renderer::new(&c, root, cfg.font_path.as_deref(), font_size, width, height)?;
    let gc = c.generate_id()?;
    c.create_gc(gc, root, &CreateGCAux::new())?;
    let palette = &cfg.palette;
    let mut boxes = Vec::new();
    for (target, label) in targets.iter().zip(&labels) {
        let win = c.generate_id()?;
        c.create_window(
            0,
            win,
            root,
            target.x - width as i16 / 2,
            target.y - height as i16 / 2,
            width,
            height,
            2,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(palette.accent)
                .border_pixel(palette.background)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS),
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirulauncher\0NIRULANCHER\0",
        )?;
        c.map_window(win)?;
        boxes.push((win, label.clone()));
    }
    c.flush()?;
    let draw = |renderer: &mut Renderer, win: Window, label: &str, typed: &str| -> Result<()> {
        renderer.fill(
            &c,
            gc,
            Rectangle {
                x: 0,
                y: 0,
                width,
                height,
            },
            palette.accent,
        )?;
        let text_width = advance * label.chars().count() as i16;
        let x = (width as i16 - text_width) / 2;
        let baseline = pad / 2 + line_height - line_height / 5;
        // What has been typed is shown dimmed, the rest at full strength.
        let done: String = label.chars().take(typed.chars().count()).collect();
        let rest: String = label.chars().skip(typed.chars().count()).collect();
        renderer.line(&c, gc, x, baseline, &done, palette.muted, palette.accent)?;
        renderer.line(
            &c,
            gc,
            x + advance * done.chars().count() as i16,
            baseline,
            &rest,
            palette.background,
            palette.accent,
        )?;
        renderer.present(&c, gc, win, width)?;
        Ok(())
    };
    grab_keyboard(&c, root)?;
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
    let mut keymap = Keymap::load(&c);
    let mut typed = String::new();
    let mut shown: Vec<usize> = (0..boxes.len()).collect();
    let chosen = loop {
        let event = c.wait_for_event()?;
        match event {
            Event::Expose(e) if e.count == 0 => {
                if let Some(i) = boxes.iter().position(|(w, _)| *w == e.window) {
                    draw(&mut renderer, boxes[i].0, &boxes[i].1, &typed)?;
                    // The event wait does not flush; without this the label stays empty.
                    c.flush()?;
                }
            }
            Event::MappingNotify(_) => keymap = Keymap::load(&c),
            Event::ButtonPress(_) => break None,
            Event::KeyPress(key) => {
                let sym = keymap.symbol(key.detail, key.state);
                if sym == 0xff1b {
                    break None; // Escape
                }
                if sym == 0xff08 {
                    typed.pop(); // BackSpace
                } else if let Some(ch) = crate::compose::keysym_char(sym) {
                    if ch.is_control() || ch == ' ' {
                        continue;
                    }
                    let mut next = typed.clone();
                    next.extend(ch.to_lowercase());
                    let (matching, done) = narrow(&labels, &next);
                    if let Some(i) = done {
                        break Some(i);
                    }
                    if matching.is_empty() {
                        break None;
                    }
                    typed = next;
                } else {
                    // Modifiers alone do nothing; anything else closes.
                    if !(0xffe1..=0xffee).contains(&sym) && !(0xfe01..=0xfe13).contains(&sym) {
                        break None;
                    }
                    continue;
                }
                let (matching, _) = narrow(&labels, &typed);
                for (i, (win, label)) in boxes.iter().enumerate() {
                    let visible = matching.contains(&i);
                    if visible != shown.contains(&i) {
                        if visible {
                            c.map_window(*win)?;
                        } else {
                            c.unmap_window(*win)?;
                        }
                    }
                    if visible {
                        draw(&mut renderer, *win, label, &typed)?;
                    }
                }
                shown = matching;
                c.flush()?;
            }
            _ => {}
        }
    };
    let _ = c.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = c.ungrab_pointer(x11rb::CURRENT_TIME);
    if let Some(i) = chosen {
        // Pointer first: with focus following the mouse, the window under the
        // pointer when the labels close would otherwise take the focus back.
        let _ = c.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, targets[i].x, targets[i].y);
        let atom = intern(&c, "_NET_ACTIVE_WINDOW")?;
        let event = ClientMessageEvent::new(
            32,
            targets[i].window,
            atom,
            [2, x11rb::CURRENT_TIME, 0, 0, 0],
        );
        c.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?;
        c.sync()?;
    }
    for (win, _) in &boxes {
        let _ = c.destroy_window(*win);
    }
    c.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_never_share_a_prefix() {
        assert_eq!(labels(3, "letters"), ["a", "s", "d"]);
        assert_eq!(labels(3, "numbers"), ["1", "2", "3"]);
        let many = labels(30, "letters");
        assert_eq!(many.len(), 30);
        assert!(many.iter().all(|l| l.len() == 2));
        let numbers = labels(12, "numbers");
        assert_eq!(numbers[0], "01");
        assert_eq!(numbers[11], "12");
        for set in [&many, &numbers] {
            for a in set.iter() {
                assert_eq!(set.iter().filter(|b| b.starts_with(a.as_str())).count(), 1);
            }
        }
    }

    #[test]
    fn typing_narrows_and_completes() {
        let set = labels(30, "letters");
        let (matching, done) = narrow(&set, "a");
        assert_eq!(matching.len(), 26);
        assert!(done.is_none());
        let (matching, done) = narrow(&set, "as");
        assert_eq!(matching, vec![1]);
        assert_eq!(done, Some(1));
        assert!(narrow(&set, "zz").0.is_empty());
    }
}
