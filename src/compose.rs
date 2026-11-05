//! Dead keys and diacritic folding for the search field.
//!
//! The compose table and keysym rules are NIRULOCK 0.18.0's built-in fallback
//! (`src/keyboard.rs`), so `´ ` ^ ~ ¨` give the same text in the lock screen
//! and here. NIRULOCK prefers libxkbcommon through `dlopen`; this launcher
//! ships as a static musl binary, where `dlopen` cannot load system
//! libraries, so the fallback table is the whole implementation here. It
//! covers the dead keys of the Swedish and other Latin-1 layouts.

const XK_SPACE: u32 = 0x20;

pub fn is_modifier(sym: u32) -> bool {
    // Shift_L..Hyper_R, ISO_Lock..ISO_Last_Group_Lock (Level3/Level5 shifts,
    // latches and group switches), Mode_switch and Num_Lock.
    (0xffe1..=0xffee).contains(&sym)
        || (0xfe01..=0xfe13).contains(&sym)
        || sym == 0xff7e
        || sym == 0xff7f
}

fn is_dead(sym: u32) -> bool {
    (0xfe50..=0xfe8f).contains(&sym)
}

/// Keysym to character for the paths that do not use libxkbcommon's UTF-8.
pub fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        // Unicode keysyms: 0x01000000 + code point.
        0x0100_0100..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        // Keypad symbols that produce text.
        0xff80 => Some(' '),
        0xffaa => Some('*'),
        0xffab => Some('+'),
        0xffac => Some(','),
        0xffad => Some('-'),
        0xffae => Some('.'),
        0xffaf => Some('/'),
        0xffb0..=0xffb9 => char::from_u32(sym - 0xffb0 + u32::from(b'0')),
        0xffbd => Some('='),
        _ => None,
    }
    .filter(|c| !c.is_control())
}

/// Result of feeding one keysym to a compose state.
#[derive(Debug, PartialEq, Eq)]
pub enum Compose {
    /// The keysym is not part of a sequence; handle it normally.
    Pass,
    /// The keysym started or continued a sequence; it produces nothing yet.
    Composing,
    /// A complete sequence produced this character.
    Composed(char),
    /// An invalid continuation: the sequence and the keysym are discarded,
    /// exactly as libX11 and libxkbcommon do.
    Cancelled,
}

/// The fallback compose table, used only when the locale's table cannot be
/// loaded through libxkbcommon. It gives the same results as libX11's
/// `en_US.UTF-8` Compose file for the five dead keys of the Swedish and other
/// Latin-1 layouts:
///
/// * dead key + space: the spacing character libX11 gives (`'` for acute and
///   `"` for diaeresis, the literal `` ` ^ ~ `` for the others);
/// * the same dead key twice: the literal accent (`´ ` ^ ~ ¨`);
/// * dead key + a vowel, `n` or `y`: the composed Latin-1 letter in either case;
/// * any other key cancels the sequence and is discarded; modifier keys are
///   ignored while a sequence is pending. Sequences of two different dead
///   keys (three-key sequences in libX11) are not supported and cancel.
#[derive(Default)]
pub struct BuiltinCompose {
    pending: Option<u32>,
}

impl BuiltinCompose {
    pub fn reset(&mut self) {
        self.pending = None;
    }

    pub fn feed(&mut self, sym: u32) -> Compose {
        if is_modifier(sym) {
            return if self.pending.is_some() {
                Compose::Composing
            } else {
                Compose::Pass
            };
        }
        let Some(dead) = self.pending.take() else {
            if is_dead(sym) {
                self.pending = Some(sym);
                return Compose::Composing;
            }
            return Compose::Pass;
        };
        match builtin_sequence(dead, sym) {
            Some(c) => Compose::Composed(c),
            None => Compose::Cancelled,
        }
    }
}

const DEAD_GRAVE: u32 = 0xfe50;
const DEAD_ACUTE: u32 = 0xfe51;
const DEAD_CIRCUMFLEX: u32 = 0xfe52;
const DEAD_TILDE: u32 = 0xfe53;
const DEAD_DIAERESIS: u32 = 0xfe57;

fn builtin_sequence(dead: u32, sym: u32) -> Option<char> {
    if sym == XK_SPACE {
        return match dead {
            DEAD_GRAVE => Some('`'),
            DEAD_ACUTE => Some('\''),
            DEAD_CIRCUMFLEX => Some('^'),
            DEAD_TILDE => Some('~'),
            DEAD_DIAERESIS => Some('"'),
            _ => None,
        };
    }
    if sym == dead {
        return match dead {
            DEAD_GRAVE => Some('`'),
            DEAD_ACUTE => Some('\u{b4}'),
            DEAD_CIRCUMFLEX => Some('^'),
            DEAD_TILDE => Some('~'),
            DEAD_DIAERESIS => Some('\u{a8}'),
            _ => None,
        };
    }
    let base = char::from_u32(sym).filter(char::is_ascii_alphabetic)?;
    let upper = base.is_ascii_uppercase();
    let lower = base.to_ascii_lowercase();
    let composed = match (dead, lower) {
        (DEAD_GRAVE, 'a') => 'à',
        (DEAD_GRAVE, 'e') => 'è',
        (DEAD_GRAVE, 'i') => 'ì',
        (DEAD_GRAVE, 'o') => 'ò',
        (DEAD_GRAVE, 'u') => 'ù',
        (DEAD_ACUTE, 'a') => 'á',
        (DEAD_ACUTE, 'e') => 'é',
        (DEAD_ACUTE, 'i') => 'í',
        (DEAD_ACUTE, 'o') => 'ó',
        (DEAD_ACUTE, 'u') => 'ú',
        (DEAD_ACUTE, 'y') => 'ý',
        (DEAD_CIRCUMFLEX, 'a') => 'â',
        (DEAD_CIRCUMFLEX, 'e') => 'ê',
        (DEAD_CIRCUMFLEX, 'i') => 'î',
        (DEAD_CIRCUMFLEX, 'o') => 'ô',
        (DEAD_CIRCUMFLEX, 'u') => 'û',
        (DEAD_TILDE, 'a') => 'ã',
        (DEAD_TILDE, 'n') => 'ñ',
        (DEAD_TILDE, 'o') => 'õ',
        (DEAD_DIAERESIS, 'a') => 'ä',
        (DEAD_DIAERESIS, 'e') => 'ë',
        (DEAD_DIAERESIS, 'i') => 'ï',
        (DEAD_DIAERESIS, 'o') => 'ö',
        (DEAD_DIAERESIS, 'u') => 'ü',
        (DEAD_DIAERESIS, 'y') => 'ÿ',
        _ => return None,
    };
    if !upper {
        return Some(composed);
    }
    // ÿ has no Latin-1 capital; libX11 composes U+0178.
    if composed == 'ÿ' {
        return Some('\u{178}');
    }
    composed.to_uppercase().next()
}

/// A keysym for a composed character, so the key handlers see one value.
pub fn char_keysym(ch: char) -> u32 {
    let code = u32::from(ch);
    if code <= 0xff {
        code
    } else {
        0x0100_0000 + code
    }
}

/// Function and keypad keysyms reset a pending dead key and keep their
/// meaning: Escape still closes and Enter still launches.
pub fn is_function(sym: u32) -> bool {
    (0xff00..=0xffff).contains(&sym) && !is_modifier(sym)
}

/// Fold a lowercase Latin letter with a diacritic to its base letter, one
/// character for one, so match positions stay the same in both strings.
pub fn fold_char(ch: char) -> char {
    match ch {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(syms: &[u32]) -> Vec<Compose> {
        let mut compose = BuiltinCompose::default();
        syms.iter().map(|sym| compose.feed(*sym)).collect()
    }

    fn composed(dead: u32, sym: u32) -> Option<char> {
        match feed_all(&[dead, sym]).pop() {
            Some(Compose::Composed(c)) => Some(c),
            _ => None,
        }
    }

    #[test]
    fn dead_key_and_space_match_libx11() {
        assert_eq!(composed(DEAD_GRAVE, XK_SPACE), Some('`'));
        assert_eq!(composed(DEAD_ACUTE, XK_SPACE), Some('\''));
        assert_eq!(composed(DEAD_CIRCUMFLEX, XK_SPACE), Some('^'));
        assert_eq!(composed(DEAD_TILDE, XK_SPACE), Some('~'));
        assert_eq!(composed(DEAD_DIAERESIS, XK_SPACE), Some('"'));
    }

    #[test]
    fn doubled_dead_key_gives_the_accent() {
        assert_eq!(composed(DEAD_GRAVE, DEAD_GRAVE), Some('`'));
        assert_eq!(composed(DEAD_ACUTE, DEAD_ACUTE), Some('´'));
        assert_eq!(composed(DEAD_CIRCUMFLEX, DEAD_CIRCUMFLEX), Some('^'));
        assert_eq!(composed(DEAD_TILDE, DEAD_TILDE), Some('~'));
        assert_eq!(composed(DEAD_DIAERESIS, DEAD_DIAERESIS), Some('¨'));
    }

    #[test]
    fn dead_key_and_letter_compose_in_both_cases() {
        let e = u32::from(b'e');
        assert_eq!(composed(DEAD_ACUTE, e), Some('é'));
        assert_eq!(composed(DEAD_ACUTE, u32::from(b'E')), Some('É'));
        assert_eq!(composed(DEAD_GRAVE, u32::from(b'a')), Some('à'));
        assert_eq!(composed(DEAD_CIRCUMFLEX, u32::from(b'O')), Some('Ô'));
        assert_eq!(composed(DEAD_TILDE, u32::from(b'n')), Some('ñ'));
        assert_eq!(composed(DEAD_TILDE, u32::from(b'N')), Some('Ñ'));
        assert_eq!(composed(DEAD_DIAERESIS, u32::from(b'u')), Some('ü'));
        assert_eq!(composed(DEAD_DIAERESIS, u32::from(b'y')), Some('ÿ'));
        assert_eq!(composed(DEAD_DIAERESIS, u32::from(b'Y')), Some('Ÿ'));
        assert_eq!(composed(DEAD_ACUTE, u32::from(b'Y')), Some('Ý'));
    }

    #[test]
    fn invalid_sequence_is_discarded_like_libx11() {
        assert_eq!(
            feed_all(&[DEAD_ACUTE, u32::from(b'x')]),
            [Compose::Composing, Compose::Cancelled]
        );
        // Two different dead keys cancel in the fallback table.
        assert_eq!(
            feed_all(&[DEAD_ACUTE, DEAD_TILDE]),
            [Compose::Composing, Compose::Cancelled]
        );
        // After a cancel the next key is ordinary again.
        assert_eq!(
            feed_all(&[DEAD_ACUTE, u32::from(b'x'), u32::from(b'x')]),
            [Compose::Composing, Compose::Cancelled, Compose::Pass]
        );
    }

    #[test]
    fn modifiers_do_not_break_a_sequence() {
        // dead_acute, Shift_L, E gives É.
        assert_eq!(
            feed_all(&[DEAD_ACUTE, 0xffe1, u32::from(b'E')]),
            [
                Compose::Composing,
                Compose::Composing,
                Compose::Composed('É')
            ]
        );
        // AltGr (ISO_Level3_Shift) on its own passes through.
        assert_eq!(feed_all(&[0xfe03]), [Compose::Pass]);
    }

    #[test]
    fn function_keys_are_not_compose_input() {
        assert!(is_function(0xff1b), "Escape");
        assert!(is_function(0xff0d), "Return");
        assert!(is_function(0xff08), "BackSpace");
        assert!(!is_function(0xffe1), "Shift is a modifier");
        assert!(!is_function(0xfe51), "dead_acute");
        assert!(!is_function(u32::from(b'e')));
    }

    #[test]
    fn composed_letters_become_keysyms() {
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('Ÿ'), 0x0100_0178);
        assert_eq!(keysym_char(char_keysym('Ÿ')), Some('Ÿ'));
        assert_eq!(keysym_char(0xfe51), None, "a dead key is not text");
    }

    #[test]
    fn folding_maps_diacritics_one_to_one() {
        assert_eq!(fold("éèêë"), "eeee");
        assert_eq!(fold("åäö"), "aao");
        assert_eq!(fold("ñüçøł"), "nucol");
        assert_eq!(fold("gimp"), "gimp");
        for word in ["skärmläsare", "société", "œuvre"] {
            assert_eq!(fold(word).chars().count(), word.chars().count());
        }
    }
}
