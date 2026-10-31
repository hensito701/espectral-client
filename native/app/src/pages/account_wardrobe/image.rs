//! Local image helpers (replaces lib/api.ts `fileToPngDataUrl`): read a PNG
//! the user picked, fit it into `max` px and re-encode it as PNG base64 for
//! the engine. Only PNG input is supported natively (the browser also
//! decoded JPEG / WebP).

use std::path::Path;

use base64::Engine as _;

/// Why a picked file cannot be used.
#[derive(Debug)]
pub enum ImageError {
    /// Not a PNG (label = detected kind, like UnsupportedImageError.label).
    Unsupported(String),
    Io(String),
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn kind_label(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "JPEG".into()
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "WebP".into()
    } else if bytes.starts_with(b"GIF8") {
        "GIF".into()
    } else if bytes.starts_with(b"BM") {
        "BMP".into()
    } else {
        "?".into()
    }
}

/// Read `path`, downscale into `max`x`max` (aspect kept) and return base64 PNG.
pub fn file_to_png_base64(path: &Path, max: u32) -> Result<String, ImageError> {
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    if bytes.len() < 8 || bytes[..8] != PNG_MAGIC {
        return Err(ImageError::Unsupported(kind_label(&bytes)));
    }
    let Some(buf) = crate::util::decode_png(&bytes) else {
        return Err(ImageError::Unsupported("PNG".into()));
    };
    let (w, h) = (buf.width(), buf.height());
    if w <= max && h <= max {
        // small enough: keep the original bytes (skins must stay pixel-exact)
        return Ok(base64::engine::general_purpose::STANDARD.encode(&bytes));
    }
    let scale = max as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1));
    let src = buf.as_bytes();
    let out = box_resize(src, w, h, nw, nh);
    let png = encode_png(&out, nw, nh).map_err(ImageError::Io)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}

/// Area-average downscale of an RGBA buffer.
fn box_resize(src: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let y0 = (y as u64 * h as u64 / nh as u64) as u32;
        let y1 = (((y + 1) as u64 * h as u64).div_ceil(nh as u64) as u32).clamp(y0 + 1, h);
        for x in 0..nw {
            let x0 = (x as u64 * w as u64 / nw as u64) as u32;
            let x1 = (((x + 1) as u64 * w as u64).div_ceil(nw as u64) as u32).clamp(x0 + 1, w);
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u64;
                    }
                    n += 1;
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// File name without extension, at most 40 chars (staged skin name).
pub fn stem40(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().chars().take(40).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_and_encode_roundtrip() {
        let w = 300;
        let h = 150;
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 100, 50, 255]).collect();
        let png = encode_png(&rgba, w, h).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.png");
        std::fs::write(&p, &png).unwrap();
        let b64 = file_to_png_base64(&p, 256).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let buf = crate::util::decode_png(&bytes).unwrap();
        assert_eq!((buf.width(), buf.height()), (256, 128));
        assert_eq!(buf.as_bytes()[..4], [200, 100, 50, 255]);
        std::fs::write(&p, b"\xff\xd8\xff\xe0jpeg").unwrap();
        assert!(matches!(file_to_png_base64(&p, 256), Err(ImageError::Unsupported(l)) if l == "JPEG"));
        assert_eq!(stem40(Path::new("/x/my skin.png")), "my skin");
    }
}
