//! Application icons for the panel: a desktop entry's `Icon` looked up in
//! the icon themes (hicolor, the user's own icons, pixmaps), drawn once at
//! the size the panel needs and kept in `~/.cache/nirulauncher/icons/` as
//! raw pixels, so the next opening only reads a few kilobytes per icon.
//!
//! PNG and SVG are read; anything else (XPM) gets the panel's letter tile.
use std::path::{Path, PathBuf};

/// Sizes in the order they are tried for a wanted size: the nearest larger
/// one first (downscaling looks better), then smaller ones.
const SIZES: [u32; 12] = [16, 22, 24, 32, 36, 48, 64, 72, 96, 128, 256, 512];

/// A premultiplied 0xAARRGGBB picture, `size` x `size`.
pub(crate) struct Picture {
    pub(crate) size: u16,
    pub(crate) pixels: Vec<u32>,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => out.push(PathBuf::from(d)),
        None => out.extend(home.as_ref().map(|h| h.join(".local/share"))),
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    out
}

/// Where an icon name can be, best first, for a wanted pixel size.
fn candidates(name: &str, want: u32) -> Vec<PathBuf> {
    let mut sizes: Vec<u32> = SIZES.iter().copied().filter(|s| *s >= want).collect();
    sizes.extend(SIZES.iter().rev().copied().filter(|s| *s < want));
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".icons"));
    }
    roots.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    let mut out = Vec::new();
    for root in &roots {
        let theme = root.join("hicolor");
        for s in &sizes {
            out.push(theme.join(format!("{s}x{s}/apps/{name}.png")));
        }
        out.push(theme.join(format!("scalable/apps/{name}.svg")));
        out.push(root.join(format!("{name}.png")));
        out.push(root.join(format!("{name}.svg")));
    }
    for dir in data_dirs() {
        out.push(dir.join(format!("pixmaps/{name}.png")));
        out.push(dir.join(format!("pixmaps/{name}.svg")));
    }
    out
}

/// The file behind an `Icon` value, if there is one we can draw.
pub(crate) fn find(icon: &str, want: u32) -> Option<PathBuf> {
    let icon = icon.trim();
    if icon.is_empty() {
        return None;
    }
    if icon.starts_with('/') {
        let p = PathBuf::from(icon);
        return (p.is_file() && drawable(&p)).then_some(p);
    }
    // A name never carries a directory; one that does is not looked up.
    if icon.contains('/') {
        return None;
    }
    candidates(icon, want).into_iter().find(|p| p.is_file())
}

fn drawable(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "svg")
    )
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache")
        })
        .join("nirulauncher/icons")
}

/// FNV-1a over the file's path, size and modification time, and the
/// drawn size: a changed icon file is drawn again.
fn cache_key(path: &Path, size: u16) -> String {
    let meta = std::fs::metadata(path).ok();
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}-{len}-{mtime}-{size}.argb")
}

/// The picture for an `Icon` value at `size` pixels: from the cache, else
/// drawn and cached. `None` when there is no drawable icon.
pub(crate) fn load(icon: &str, size: u16) -> Option<Picture> {
    let path = find(icon, u32::from(size))?;
    let cache = cache_dir().join(cache_key(&path, size));
    if let Ok(bytes) = std::fs::read(&cache) {
        // An empty file: drawn before and found empty (see below).
        if bytes.is_empty() {
            return None;
        }
        if bytes.len() == usize::from(size) * usize::from(size) * 4 {
            let pixels = bytes
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            return Some(Picture { size, pixels });
        }
    }
    // An icon that comes out (nearly) empty, such as one drawn with text
    // (the panel draws no text in icons), gets the letter tile instead,
    // and that is remembered as an empty cache file.
    let picture = draw(&path, size).filter(|p| visible(&p.pixels));
    let bytes: Vec<u8> = picture
        .as_ref()
        .map(|p| p.pixels.iter().flat_map(|p| p.to_le_bytes()).collect())
        .unwrap_or_default();
    if std::fs::create_dir_all(cache_dir()).is_ok() {
        let tmp = cache.with_extension("tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &cache);
        }
    }
    picture
}

/// Whether enough of a picture is drawn to be recognised.
fn visible(pixels: &[u32]) -> bool {
    let solid = pixels.iter().filter(|p| *p >> 24 > 64).count();
    solid * 100 >= pixels.len().max(1) * 6
}

fn draw(path: &Path, size: u16) -> Option<Picture> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "svg" => draw_svg(&std::fs::read(path).ok()?, size),
        "png" => draw_png(path, size),
        _ => None,
    }
}

fn draw_svg(data: &[u8], size: u16) -> Option<Picture> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(u32::from(size), u32::from(size))?;
    let s = tree.size();
    let scale = f32::from(size) / s.width().max(s.height()).max(1.0);
    // Centred when the drawing is not square.
    let dx = (f32::from(size) - s.width() * scale) / 2.0;
    let dy = (f32::from(size) - s.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia keeps premultiplied RGBA, which is what the renderer takes.
    let pixels = pixmap
        .data()
        .chunks_exact(4)
        .map(|p| {
            u32::from(p[3]) << 24 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2])
        })
        .collect();
    Some(Picture { size, pixels })
}

fn draw_png(path: &Path, size: u16) -> Option<Picture> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let rgba: Vec<[u8; 4]> = match info.color_type {
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    if rgba.len() < w * h {
        return None;
    }
    Some(Picture {
        size,
        pixels: scale(&rgba, w, h, usize::from(size)),
    })
}

/// Area-average a straight-alpha picture into a premultiplied square,
/// keeping its aspect and centring it.
fn scale(src: &[[u8; 4]], w: usize, h: usize, size: usize) -> Vec<u32> {
    let mut out = vec![0u32; size * size];
    let factor = size as f64 / w.max(h) as f64;
    let (dw, dh) = (
        ((w as f64 * factor).round() as usize).clamp(1, size),
        ((h as f64 * factor).round() as usize).clamp(1, size),
    );
    let (ox, oy) = ((size - dw) / 2, (size - dh) / 2);
    for y in 0..dh {
        let sy0 = y * h / dh;
        let sy1 = ((y + 1) * h / dh).max(sy0 + 1).min(h);
        for x in 0..dw {
            let sx0 = x * w / dw;
            let sx1 = ((x + 1) * w / dw).max(sx0 + 1).min(w);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let p = src[sy * w + sx];
                    let pa = u64::from(p[3]);
                    // Premultiply before averaging, or edges go dark.
                    r += u64::from(p[0]) * pa;
                    g += u64::from(p[1]) * pa;
                    b += u64::from(p[2]) * pa;
                    a += pa;
                    n += 1;
                }
            }
            let n = n.max(1);
            let px = |c: u64| ((c / 255) / n).min(255) as u32;
            out[(oy + y) * size + ox + x] =
                ((a / n).min(255) as u32) << 24 | px(r) << 16 | px(g) << 8 | px(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_larger_size_is_tried_before_a_smaller_one() {
        let c = candidates("firefox", 48);
        let first = c[0].to_string_lossy().to_string();
        assert!(first.ends_with("hicolor/48x48/apps/firefox.png"), "{first}");
        let i64 = c
            .iter()
            .position(|p| p.ends_with("hicolor/64x64/apps/firefox.png"));
        let i32 = c
            .iter()
            .position(|p| p.ends_with("hicolor/32x32/apps/firefox.png"));
        assert!(i64 < i32);
        assert!(find("../etc/passwd", 48).is_none());
        assert!(find("", 48).is_none());
    }

    #[test]
    fn scaling_keeps_the_aspect_and_premultiplies() {
        // 4x2 opaque red over transparent: a 4 px square gets two rows.
        let red = [255, 0, 0, 255];
        let px = scale(&[red; 8], 4, 2, 4);
        assert_eq!(px[0], 0); // the top row is empty
        assert_eq!(px[4], 0xffff_0000);
        assert!(visible(&px) && !visible(&[0u32; 100]));
        // Half transparent white is premultiplied to grey.
        let half = scale(&[[255, 255, 255, 128]], 1, 1, 1);
        assert_eq!(half[0] >> 24, 128);
        assert_eq!(half[0] & 0xff, 128);
    }

    #[test]
    fn an_svg_is_drawn_and_cached() {
        let _lock = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("nirupanel-icons-{}", std::process::id()));
        let apps = dir.join("data/icons/hicolor/scalable/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("niru-test.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#00ff00"/></svg>"##,
        )
        .unwrap();
        let (data, cache) = (
            std::env::var_os("XDG_DATA_HOME"),
            std::env::var_os("XDG_CACHE_HOME"),
        );
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
        std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
        let p = load("niru-test", 16).expect("drawn");
        assert_eq!(p.pixels.len(), 256);
        assert_eq!(p.pixels[8 * 16 + 8], 0xff00_ff00);
        let cached = std::fs::read_dir(dir.join("cache/nirulauncher/icons"))
            .unwrap()
            .count();
        assert_eq!(cached, 1);
        // The second time it comes from the cache.
        let again = load("niru-test", 16).unwrap();
        assert_eq!(again.pixels, p.pixels);
        match data {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        match cache {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
