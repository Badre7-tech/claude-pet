//! "Regarde mon écran": a picture of the monitor under the mouse, saved in the brain folder
//! where Claude can read it (and nothing else) for that one message.

use crate::launcher::normalize;
use std::{fs, io::BufWriter, path::Path};

/// Where the picture goes, relative to the brain folder (Claude may only read inside it).
pub const FILE: &str = "screens/ecran.png";
/// Claude sees images best up to about this size; bigger only costs more.
const MAX_SIDE: usize = 1568;

/// Is the human asking the pet to look at their screen?
pub fn wanted(message: &str) -> bool {
    let text = normalize(message);
    let words: Vec<&str> = text.split(' ').collect();
    let mentions_screen = words.iter().any(|w| ["ecran", "ecrans", "screen", "screenshot"].contains(w));
    let looks = words
        .iter()
        .any(|w| ["regarde", "regardes", "vois", "voir", "lis", "lire", "check", "capture", "analyse", "aide", "aider"].contains(w));
    mentions_screen && (looks || text.contains("sur mon ecran") || text.contains("capture d ecran"))
}

/// Averages blocks of pixels so the long side is at most `max`.
fn shrink(rgb: &[u8], w: usize, h: usize, max: usize) -> (Vec<u8>, usize, usize) {
    if w.max(h) <= max {
        return (rgb.to_vec(), w, h);
    }
    let scale = max as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1));
    let mut out = vec![0u8; nw * nh * 3];
    for oy in 0..nh {
        let (y0, y1) = (oy * h / nh, ((oy + 1) * h / nh).max(oy * h / nh + 1));
        for ox in 0..nw {
            let (x0, x1) = (ox * w / nw, ((ox + 1) * w / nw).max(ox * w / nw + 1));
            let mut sum = [0u32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 3;
                    for c in 0..3 {
                        sum[c] += rgb[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (oy * nw + ox) * 3;
            for c in 0..3 {
                out[o + c] = (sum[c] / n) as u8;
            }
        }
    }
    (out, nw, nh)
}

fn save_png(rgb: &[u8], w: usize, h: usize, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let (data, w, h) = shrink(rgb, w, h, MAX_SIDE);
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&data).map_err(|e| e.to_string())
}

/// Captures the rectangle (in physical pixels of the whole desktop) into `brain/screens/ecran.png`.
pub fn capture(brain: &Path, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let rgb = grab(x, y, w, h)?;
    save_png(&rgb, w as usize, h as usize, &brain.join(FILE))
}

#[cfg(not(windows))]
fn grab(_: i32, _: i32, _: i32, _: i32) -> Result<Vec<u8>, String> {
    Err("je ne sais regarder l'écran que sous Windows".into())
}

#[cfg(windows)]
fn grab(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>, String> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
    };
    if w <= 0 || h <= 0 {
        return Err("écran introuvable".into());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("pas d'accès à l'écran".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(memory, bitmap.into());
        let copied = BitBlt(memory, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h; // top row first
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;
        let mut bgra = vec![0u8; w as usize * h as usize * 4];
        let rows = GetDIBits(memory, bitmap, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        if copied.is_err() || rows == 0 {
            return Err("la capture d'écran a échoué".into());
        }
        Ok(bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_when_to_look() {
        assert!(wanted("regarde mon écran"));
        assert!(wanted("Pyrosan tu vois quoi sur mon écran ?"));
        assert!(wanted("aide-moi avec ce qu'il y a à l'écran"));
        assert!(wanted("fais une capture d'écran et dis-moi ce qui cloche"));
        assert!(!wanted("quel écran OLED acheter ?"));
        assert!(!wanted("regarde la météo"));
    }

    #[test]
    fn big_screens_are_shrunk_and_saved() {
        let (w, h) = (2560, 1440);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| (i % 251) as u8).collect();
        let (small, sw, sh) = shrink(&rgb, w, h, MAX_SIDE);
        assert_eq!((sw, sh), (1568, 882));
        assert_eq!(small.len(), sw * sh * 3);
        let dir = std::env::temp_dir().join(format!("claude-pet-screen-{}", crate::stats::now_ms()));
        save_png(&rgb, w, h, &dir.join(FILE)).unwrap();
        let bytes = fs::read(dir.join(FILE)).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        fs::remove_dir_all(dir).unwrap();
    }
}
