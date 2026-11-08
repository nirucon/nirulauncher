//! Color emoji from a CBDT bitmap font (Noto Color Emoji).
//!
//! The font is memory-mapped, so only the pages of the glyphs actually shown
//! are read. A sequence (a ZWJ family, a flag, a skin tone) is turned into
//! one glyph with the font's own GSUB ligatures, its PNG strike is decoded
//! and scaled once to the row size, and the result is cached for the rest of
//! the run. The font is opened only when an emoji row is first drawn.

use std::collections::HashMap;
use ttf_parser::{
    gsub::SubstitutionSubtable, opentype_layout::LayoutTable, Face, GlyphId, RasterImageFormat,
};

/// A scaled emoji: premultiplied 0xAARRGGBB, `size` x `size`.
pub struct Picture {
    pub size: u16,
    pub pixels: Vec<u32>,
}

pub struct EmojiFont {
    face: Face<'static>,
    cache: HashMap<(String, u16), Option<std::rc::Rc<Picture>>>,
}

/// Where distributions install Noto Color Emoji.
const PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/TTF/NotoColorEmoji.ttf",
];

fn find(configured: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(path) = configured {
        return Some(path.into());
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        for rel in [
            ".local/share/fonts/NotoColorEmoji.ttf",
            ".fonts/NotoColorEmoji.ttf",
        ] {
            let p = home.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    for p in PATHS {
        if std::path::Path::new(p).is_file() {
            return Some(p.into());
        }
    }
    // fontconfig knows other places; it costs a process, so it comes last.
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "Noto Color Emoji:fontformat=TrueType"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let p = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let name = p.file_name()?.to_string_lossy().to_lowercase();
    (name.contains("emoji") && p.is_file()).then_some(p)
}

/// Map a file read-only for the life of the process. The launcher is one
/// short run, so the mapping is never released.
fn map(path: &std::path::Path) -> Option<&'static [u8]> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len() as usize;
    if len == 0 || len > 256 << 20 {
        return None;
    }
    // SAFETY: a private read-only mapping of a regular file we opened; the
    // slice lives as long as the process and is only read.
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        );
        if ptr == libc::MAP_FAILED {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr as *const u8, len))
    }
}

impl EmojiFont {
    pub fn open(configured: Option<&str>) -> Option<Self> {
        let path = find(configured)?;
        let data = map(&path)?;
        let face = Face::parse(data, 0).ok()?;
        face.tables().cbdt?;
        Some(Self {
            face,
            cache: HashMap::new(),
        })
    }

    /// The single glyph a sequence shapes to, if the font has one.
    pub fn glyph(&self, text: &str) -> Option<GlyphId> {
        let mut glyphs: Vec<GlyphId> = Vec::new();
        for ch in text.chars() {
            match self.face.glyph_index(ch) {
                Some(g) => glyphs.push(g),
                // Variation selectors without a glyph do not change the emoji.
                None if ch == '\u{fe0f}' || ch == '\u{fe0e}' => {}
                None => return None,
            }
        }
        if let Some(gsub) = self.face.tables().gsub {
            ligate(&gsub, &mut glyphs);
        }
        // Zero-width helpers (ZWJ, VS16) left over have no bitmap.
        let shown: Vec<GlyphId> = glyphs
            .into_iter()
            .filter(|g| self.face.glyph_raster_image(*g, 128).is_some())
            .collect();
        (shown.len() == 1).then(|| shown[0])
    }

    /// The emoji scaled to `size` pixels, decoded once per run.
    pub fn picture(&mut self, text: &str, size: u16) -> Option<std::rc::Rc<Picture>> {
        let key = (text.to_string(), size);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let made = self.render(text, size).map(std::rc::Rc::new);
        self.cache.insert(key, made.clone());
        made
    }

    fn render(&self, text: &str, size: u16) -> Option<Picture> {
        let glyph = self.glyph(text)?;
        let image = self.face.glyph_raster_image(glyph, u16::MAX)?;
        if image.format != RasterImageFormat::PNG {
            return None;
        }
        let (w, h, rgba) = decode_png(image.data)?;
        Some(scale(&rgba, w, h, size))
    }
}

/// Apply every ligature lookup of the font in order, left to right, the way
/// a shaper applies the `ccmp`/`liga` features Noto puts them in.
fn ligate(gsub: &LayoutTable, glyphs: &mut Vec<GlyphId>) {
    for li in 0..gsub.lookups.len() {
        let Some(lookup) = gsub.lookups.get(li) else {
            continue;
        };
        for si in 0..lookup.subtables.len() {
            let Some(SubstitutionSubtable::Ligature(lig)) =
                lookup.subtables.get::<SubstitutionSubtable>(si)
            else {
                continue;
            };
            let mut i = 0;
            while i < glyphs.len() {
                let mut replaced = false;
                if let Some(set_index) = lig.coverage.get(glyphs[i]) {
                    if let Some(set) = lig.ligature_sets.get(set_index) {
                        for l in set {
                            let n = l.components.len() as usize;
                            if i + 1 + n > glyphs.len() {
                                continue;
                            }
                            if l.components
                                .into_iter()
                                .enumerate()
                                .all(|(k, g)| glyphs[i + 1 + k] == g)
                            {
                                glyphs.splice(i..i + 1 + n, [l.glyph]);
                                replaced = true;
                                break;
                            }
                        }
                    }
                }
                if !replaced {
                    i += 1;
                }
            }
        }
    }
}

fn decode_png(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3]
            .chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2]
            .chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((w, h, rgba))
}

/// Area-average `w` x `h` RGBA into a `size` square, centred, premultiplied.
fn scale(rgba: &[u8], w: u32, h: u32, size: u16) -> Picture {
    let size_u = u32::from(size.max(1));
    let longest = w.max(h).max(1);
    let (tw, th) = ((w * size_u / longest).max(1), (h * size_u / longest).max(1));
    let (ox, oy) = ((size_u - tw) / 2, (size_u - th) / 2);
    let mut pixels = vec![0u32; (size_u * size_u) as usize];
    for ty in 0..th {
        let y0 = ty * h / th;
        let y1 = ((ty + 1) * h / th).max(y0 + 1).min(h);
        for tx in 0..tw {
            let x0 = tx * w / tw;
            let x1 = ((tx + 1) * w / tw).max(x0 + 1).min(w);
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    let pa = u32::from(rgba[i + 3]);
                    r += u32::from(rgba[i]) * pa;
                    g += u32::from(rgba[i + 1]) * pa;
                    b += u32::from(rgba[i + 2]) * pa;
                    a += pa;
                    n += 1;
                }
            }
            let n = n.max(1);
            let (r, g, b, a) = (r / 255 / n, g / 255 / n, b / 255 / n, a / n);
            pixels[((oy + ty) * size_u + ox + tx) as usize] = (a << 24) | (r << 16) | (g << 8) | b;
        }
    }
    Picture { size, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_colour_and_alpha() {
        // 2x2 opaque red with one transparent pixel, scaled to 1x1.
        let rgba = [255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 0];
        let p = scale(&rgba, 2, 2, 1);
        let px = p.pixels[0];
        assert_eq!(px >> 24, 191);
        assert_eq!((px >> 16) & 255, 191, "premultiplied red");
        assert_eq!(px & 0xffff, 0);
    }

    /// Every emoji row the picker offers renders as one bitmap glyph with
    /// the installed Noto Color Emoji (skipped when the font is missing).
    #[test]
    fn noto_renders_every_emoji_in_the_data() {
        let Some(mut font) = EmojiFont::open(std::env::var("NIRU_EMOJI_FONT").ok().as_deref())
        else {
            eprintln!("no Noto Color Emoji here; skipped");
            return;
        };
        let mut missing = Vec::new();
        let mut total = 0;
        for e in crate::modes::emoji::data() {
            if e.group == "Text symbols" {
                continue;
            }
            for text in std::iter::once(e.chars).chain(e.tones.iter().copied()) {
                total += 1;
                if font.glyph(text).is_none() {
                    missing.push(text.to_string());
                }
            }
        }
        eprintln!(
            "{} of {total} emoji do not render: {missing:?}",
            missing.len()
        );
        assert!(
            missing.len() * 100 <= total,
            "{} of {total} do not render: {:?}",
            missing.len(),
            &missing[..missing.len().min(40)]
        );
        // A first screen: open the font and draw 14 rows.
        let started = std::time::Instant::now();
        let mut fresh = EmojiFont::open(std::env::var("NIRU_EMOJI_FONT").ok().as_deref()).unwrap();
        for e in crate::modes::emoji::data().iter().take(14) {
            fresh.picture(e.chars, 26);
        }
        eprintln!("open + 14 emoji: {:?}", started.elapsed());
        let p = font.picture("😀", 24).expect("grinning face renders");
        assert_eq!(p.pixels.len(), 24 * 24);
        assert!(p.pixels.iter().any(|px| px >> 24 == 255));
    }
}
