//! Modrinth project icons (cdn.modrinth.com): fetched with the engine's HTTP
//! client on the tokio runtime, decoded + downscaled off the UI thread
//! (png/jpeg/webp/gif → at most 88×88, the 44px tile at 2×), turned into a
//! `slint::Image` on the UI thread and kept in a small in-memory LRU.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::bridge::Ctx;
use crate::AppWindow;

/// Decoded icons kept in memory (88×88 RGBA ≈ 30 KB each → ≈ 4 MB max).
const CAPACITY: usize = 128;
/// Icons bigger than this are skipped (MonogramTile fallback).
const MAX_BYTES: usize = 4 * 1024 * 1024;
const EDGE: u32 = 88;

struct Lru {
    entries: VecDeque<(String, Image)>,
}

impl Lru {
    fn get(&mut self, url: &str) -> Option<Image> {
        let pos = self.entries.iter().position(|(u, _)| u == url)?;
        let entry = self.entries.remove(pos)?;
        let img = entry.1.clone();
        self.entries.push_front(entry);
        Some(img)
    }

    fn put(&mut self, url: String, img: Image) {
        if let Some(pos) = self.entries.iter().position(|(u, _)| *u == url) {
            self.entries.remove(pos);
        }
        self.entries.push_front((url, img));
        while self.entries.len() > CAPACITY {
            self.entries.pop_back();
        }
    }
}

thread_local! {
    static CACHE: RefCell<Lru> = const { RefCell::new(Lru { entries: VecDeque::new() }) };
    /// URLs in flight or known-bad (no retry loop on broken icons).
    static PENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static FAILED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Cached icon (UI thread). Touches the LRU order.
pub fn get(url: &str) -> Option<Image> {
    CACHE.with(|c| c.borrow_mut().get(url))
}

/// Start loading `url` unless cached, in flight or failed before; `ready` runs
/// on the UI thread once the icon is in the cache.
pub fn request(ctx: &Ctx, url: &str, ready: fn(&AppWindow, &str)) {
    if url.is_empty() || get(url).is_some() {
        return;
    }
    if FAILED.with(|f| f.borrow().contains(url)) {
        return;
    }
    let fresh = PENDING.with(|p| p.borrow_mut().insert(url.to_string()));
    if !fresh {
        return;
    }
    let engine = ctx.engine().clone();
    let c = ctx.clone();
    let url = url.to_string();
    ctx.spawn(async move {
        let bytes = fetch(&engine, &url).await;
        let buf = match bytes {
            Some(b) => tokio::task::spawn_blocking(move || decode(&b)).await.ok().flatten(),
            None => None,
        };
        c.on_ui(move |ui| {
            PENDING.with(|p| p.borrow_mut().remove(&url));
            match buf {
                Some(buf) => {
                    CACHE.with(|c| c.borrow_mut().put(url.clone(), Image::from_rgba8(buf)));
                    ready(ui, &url);
                }
                None => {
                    FAILED.with(|f| f.borrow_mut().insert(url));
                }
            }
        });
    });
}

async fn fetch(engine: &espectral_engine::Engine, url: &str) -> Option<Vec<u8>> {
    let resp = engine.http().client().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (bytes.len() <= MAX_BYTES).then(|| bytes.to_vec())
}

/// Decode any supported format and fit it in EDGE×EDGE (keeps aspect ratio).
pub fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > EDGE || img.height() > EDGE {
        img.resize(EDGE, EDGE, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_downscales_png() {
        let mut out = Vec::new();
        let big = image::RgbaImage::from_pixel(256, 128, image::Rgba([10, 20, 30, 255]));
        image::DynamicImage::ImageRgba8(big)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        let buf = decode(&out).unwrap();
        assert_eq!((buf.width(), buf.height()), (88, 44));
        assert!(decode(b"not an image").is_none());
    }

    #[test]
    fn lru_evicts_oldest() {
        let mut l = Lru { entries: VecDeque::new() };
        for i in 0..(CAPACITY + 5) {
            l.put(format!("u{i}"), Image::default());
        }
        assert_eq!(l.entries.len(), CAPACITY);
        assert!(l.get("u0").is_none());
        assert!(l.get(&format!("u{}", CAPACITY + 4)).is_some());
    }
}
