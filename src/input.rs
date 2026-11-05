//! Keyboard input: the cached keymap, levels, Caps Lock and text keys.

use crate::*;

/// Ctrl+Backspace: drop the last word and the spaces after it.
pub(crate) fn delete_word(text: &mut String) {
    let kept = text.trim_end().len();
    text.truncate(kept);
    let word = text
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    text.truncate(word);
}

pub(crate) fn is_text_key(key: u32) -> bool {
    // Latin-1 printable range covers the launcher's documented input support,
    // plus Unicode keysyms for composed letters outside Latin-1 (Ÿ).
    // Modifier and function keysyms (0xfe00+) are never search text; 0xffe3
    // (Control_L) is also the printable U+FFE3 character, which caused Ctrl to
    // clear the menu.
    (0x20..=0xff).contains(&key) || (0x0100_0100..=0x0110_ffff).contains(&key)
}

/// The text of a key the handlers accept as text.
pub(crate) fn key_char(key: u32) -> Option<char> {
    if is_text_key(key) {
        compose::keysym_char(key)
    } else {
        None
    }
}

/// Keyboard level: base, Shift, AltGr (MOD5), then Shift+AltGr.
pub(crate) fn keyboard_level(state: KeyButMask) -> usize {
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
pub(crate) fn apply_caps(sym: u32, lock: bool) -> u32 {
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

/// Pick a keycode's keysym for a modifier state.
///
/// An XKB server's core map lists group 1 as base and Shift at 0 and 1, the
/// second group at 2 and 3, and group 1's AltGr levels at 4 and 5 (`2
/// quotedbl 2 quotedbl at twosuperior` on the Swedish layout). A plain
/// four-column core map keeps AltGr at 2 and 3. A level the key does not have
/// gives its base level, as XKB does for keys without an AltGr symbol.
pub(crate) fn keyboard_symbol(syms: &[u32], state: KeyButMask) -> u32 {
    let level = keyboard_level(state);
    let index = if level >= 2 && syms.len() > 4 {
        level + 2
    } else {
        level
    };
    let mut sym = syms.get(index).copied().unwrap_or(0);
    if sym == 0 || sym == 0xfe03 {
        sym = syms
            .get(level % 2)
            .copied()
            .filter(|sym| *sym != 0)
            .or_else(|| syms.first().copied())
            .unwrap_or(0);
    }
    apply_caps(sym, state.contains(KeyButMask::LOCK))
}

/// The core keyboard map, fetched once and again after `MappingNotify`
/// instead of one round trip per key press.
pub(crate) struct Keymap {
    pub(crate) min_keycode: u8,
    pub(crate) per_keycode: usize,
    pub(crate) keysyms: Vec<u32>,
}

impl Keymap {
    pub(crate) fn load(c: &RustConnection) -> Self {
        let min_keycode = c.setup().min_keycode;
        let max_keycode = c.setup().max_keycode;
        let reply = c
            .get_keyboard_mapping(min_keycode, max_keycode.saturating_sub(min_keycode) + 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        match reply {
            Some(reply) => Self {
                min_keycode,
                per_keycode: usize::from(reply.keysyms_per_keycode),
                keysyms: reply.keysyms,
            },
            None => Self {
                min_keycode,
                per_keycode: 0,
                keysyms: Vec::new(),
            },
        }
    }

    pub(crate) fn keysyms(&self, keycode: u8) -> &[u32] {
        let Some(index) = keycode.checked_sub(self.min_keycode) else {
            return &[];
        };
        let start = usize::from(index) * self.per_keycode;
        self.keysyms
            .get(start..start + self.per_keycode)
            .unwrap_or(&[])
    }

    pub(crate) fn symbol(&self, keycode: u8, state: KeyButMask) -> u32 {
        keyboard_symbol(self.keysyms(keycode), state)
    }
}

#[cfg(test)]
mod input_tests {
    use crate::{apply_caps, keyboard_level, keyboard_symbol};

    use x11rb::protocol::xproto::KeyButMask;

    #[test]
    fn levels_cover_shift_and_altgr() {
        assert_eq!(keyboard_level(KeyButMask::default()), 0);
        assert_eq!(keyboard_level(KeyButMask::SHIFT), 1);
        assert_eq!(keyboard_level(KeyButMask::MOD5), 2);
        assert_eq!(keyboard_level(KeyButMask::SHIFT | KeyButMask::MOD5), 3);
    }

    #[test]
    fn modifier_keys_are_not_search_text() {
        assert!(!crate::is_text_key(0xffe3), "Ctrl is not text");
        assert!(!crate::is_text_key(0xffe1), "Shift is not text");
        assert!(!crate::is_text_key(0xff52), "Arrow is not text");
        assert!(crate::is_text_key(0x70), "p is text");
        assert!(crate::is_text_key(0xe5), "å is text");
    }

    #[test]
    fn altgr_uses_the_xkb_core_columns() {
        // othala, setxkbmap se: `2 quotedbl 2 quotedbl at twosuperior at`.
        let two = [0x32, 0x22, 0x32, 0x22, 0x40, 0xb2, 0x40];
        assert_eq!(keyboard_symbol(&two, KeyButMask::default()), 0x32);
        assert_eq!(keyboard_symbol(&two, KeyButMask::SHIFT), 0x22);
        assert_eq!(
            keyboard_symbol(&two, KeyButMask::MOD5),
            0x40,
            "AltGr+2 is @"
        );
        assert_eq!(
            keyboard_symbol(&two, KeyButMask::SHIFT | KeyButMask::MOD5),
            0xb2
        );
        // `e E e E EuroSign cent EuroSign`.
        let e = [0x65, 0x45, 0x65, 0x45, 0x20ac, 0xa2, 0x20ac];
        assert_eq!(keyboard_symbol(&e, KeyButMask::MOD5), 0x20ac);
        // AltGr+ä gives the dead diaeresis; ¨ composes from there.
        let diaeresis = [0xfe57, 0xfe52, 0xfe57, 0xfe52, 0xfe53, 0xfe5a, 0xfe53];
        assert_eq!(keyboard_symbol(&diaeresis, KeyButMask::MOD5), 0xfe53);
        // A key without an AltGr level gives its base level, not nothing.
        let q = [0x71, 0x51, 0x71, 0x51, 0, 0, 0];
        assert_eq!(keyboard_symbol(&q, KeyButMask::MOD5), 0x71);
        assert_eq!(
            keyboard_symbol(&q, KeyButMask::SHIFT | KeyButMask::MOD5),
            0x51
        );
    }

    #[test]
    fn ctrl_backspace_deletes_one_word() {
        let mut text = String::from("open the café  ");
        crate::delete_word(&mut text);
        assert_eq!(text, "open the ");
        crate::delete_word(&mut text);
        assert_eq!(text, "open ");
        crate::delete_word(&mut text);
        assert_eq!(text, "");
        crate::delete_word(&mut text);
        assert_eq!(text, "");
    }

    #[test]
    fn composed_and_unicode_letters_are_text() {
        assert_eq!(crate::key_char(0xe9), Some('é'));
        assert_eq!(crate::key_char(0x0100_0178), Some('Ÿ'));
        assert_eq!(
            crate::key_char(0xfe51),
            None,
            "dead_acute itself is not text"
        );
        assert_eq!(crate::key_char(0xffe3), None);
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
