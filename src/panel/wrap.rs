//! Text that fits: the panel's font is monospaced (every character is one
//! `Renderer::advance` wide), so a width in pixels is a number of
//! characters. Text is broken between words; a word longer than a line is
//! split, and "…" ends the text only when it does not fit in the lines it
//! may take. A no-break space (`NBSP`) keeps a figure and its unit together
//! ("36 d 10 h") and is drawn as a space.

/// Joins words that stay on one line.
pub(crate) const NBSP: char = '\u{a0}';

/// `text` with its spaces made no-break spaces.
pub(crate) fn together(text: &str) -> String {
    text.replace(' ', "\u{a0}")
}

/// `text` in at most `max_lines` lines of at most `width` characters.
pub(crate) fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let width = width.max(1);
    let max_lines = max_lines.max(1);
    // Words. A word a little longer than a line ends in "…" on its own
    // line ("LibreOffi…"); a much longer one (an address, a path) is split
    // into pieces that fit.
    let mut pieces: Vec<Vec<char>> = Vec::new();
    for word in text
        .split(|c: char| c.is_whitespace() && c != NBSP)
        .filter(|w| !w.is_empty())
    {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > width && chars.len() <= width * 2 && width > 1 {
            let mut cut: Vec<char> = chars[..width - 1].to_vec();
            cut.push('…');
            pieces.push(cut);
        } else {
            pieces.extend(chars.chunks(width).map(<[char]>::to_vec));
        }
    }
    let mut lines: Vec<Vec<char>> = Vec::new();
    for piece in pieces {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + piece.len() <= width => {
                line.push(' ');
                line.extend(piece);
            }
            _ => lines.push(piece),
        }
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            if last.len() >= width {
                match last.iter().rposition(|&c| c == ' ') {
                    // Leave out the last word rather than cut it.
                    Some(space) if space > 0 => last.truncate(space),
                    _ => last.truncate(width - 1),
                }
            }
            last.push('…');
        }
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
        .into_iter()
        .map(|l| {
            l.into_iter()
                .map(|c| if c == NBSP { ' ' } else { c })
                .collect()
        })
        .collect()
}

/// Lines for a block that holds `budget` lines: each entry wrapped to at
/// most `per_entry` lines, every entry kept on at least one line while
/// there is room, and the entries that no longer fit left out.
pub(crate) fn fit<C: Copy>(
    entries: &[(String, C)],
    width: usize,
    budget: usize,
    per_entry: usize,
) -> Vec<(String, C)> {
    let mut out = Vec::new();
    for (k, (text, colour)) in entries.iter().enumerate() {
        let left = budget.saturating_sub(out.len());
        if left == 0 {
            break;
        }
        // Leave one line for each entry after this one.
        let later = (entries.len() - k - 1).min(left - 1);
        let lines = per_entry.min(left - later).max(1);
        out.extend(wrap(text, width, lines).into_iter().map(|l| (l, *colour)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_breaks_between_words() {
        assert_eq!(
            wrap("Firefox Web Browser", 12, 2),
            ["Firefox Web", "Browser"]
        );
        assert_eq!(wrap("short", 12, 2), ["short"]);
        assert_eq!(wrap("  many   spaces  here ", 20, 2), ["many spaces here"]);
        assert_eq!(wrap("", 10, 2), [""]);
        // No-break spaces hold a figure together and come out as spaces.
        let up = format!("63 °C · {}", together("up 36 d 10 h"));
        assert_eq!(wrap(&up, 14, 2), ["63 °C ·", "up 36 d 10 h"]);
        // Exactly the width.
        assert_eq!(wrap("abcd efgh", 4, 2), ["abcd", "efgh"]);
    }

    #[test]
    fn the_last_line_ends_in_an_ellipsis_only_when_text_is_left() {
        assert_eq!(wrap("one two three four five", 9, 2), ["one two", "three…"]);
        assert_eq!(wrap("one two three", 7, 1), ["one…"]);
        assert_eq!(wrap("one two", 7, 1), ["one two"]);
        // A line that ends a word exactly still gets its mark.
        assert_eq!(wrap("abc def ghi", 3, 2), ["abc", "de…"]);
        for (text, w, n) in [
            ("one two three four five six", 5, 3),
            ("aaaaaaaaaaaaaaaaaaaaaaa", 4, 2),
            ("Name day: Katarina, Katja, Carina", 10, 2),
        ] {
            let lines = wrap(text, w, n);
            assert!(lines.len() <= n, "{text}");
            assert!(lines.iter().all(|l| l.chars().count() <= w), "{lines:?}");
        }
    }

    #[test]
    fn a_long_word_is_split() {
        assert_eq!(
            wrap("https://example.org/x", 8, 3),
            ["https://", "example.", "org/x"]
        );
        assert_eq!(
            wrap("go https://example.org", 8, 3),
            ["go", "https://", "example…"]
        );
        assert_eq!(wrap("abcdefghij", 4, 2), ["abcd", "efg…"]);
        // A word a little too long keeps its start and its line.
        assert_eq!(wrap("LibreOffice Calc", 10, 2), ["LibreOffi…", "Calc"]);
        assert_eq!(wrap("Visual Studio Code", 10, 2), ["Visual", "Studio…"]);
        assert_eq!(wrap("åäö åäö", 3, 2), ["åäö", "åäö"]);
    }

    #[test]
    fn entries_share_the_lines_of_a_block() {
        let e = |t: &str, c: u8| (t.to_string(), c);
        let entries = [e("first entry is long", 1), e("second", 2), e("third", 3)];
        // Room for all: the first takes two lines.
        assert_eq!(
            fit(&entries, 12, 5, 3),
            [
                e("first entry", 1),
                e("is long", 1),
                e("second", 2),
                e("third", 3)
            ]
        );
        // Three lines: one each, the first shortened.
        assert_eq!(
            fit(&entries, 12, 3, 3),
            [e("first entry…", 1), e("second", 2), e("third", 3)]
        );
        // Fewer lines than entries: the last ones are left out.
        assert_eq!(
            fit(&entries, 12, 2, 3),
            [e("first entry…", 1), e("second", 2)]
        );
        assert!(fit(&entries, 12, 0, 3).is_empty());
    }
}
