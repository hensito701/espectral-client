//! Gallery screenshots of the Modrinth project detail modal: fetched with the
//! engine's HTTP client, decoded and shrunk off the UI thread, kept in a small
//! LRU. Same flow as `icons.rs` (which serves the 44 px project tiles), with a
//! larger target size and byte cap.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::bridge::Ctx;
use crate::AppWindow;

/// Decoded screenshots kept in memory (a 640 px RGBA image is about 1.6 MB).
const CAPACITY: usize = 16;
/// Longest edge of a decoded screenshot; the modal draws it no wider than that.
const MAX_EDGE: u32 = 640;
/// Downloads bigger than this are skipped (the modal shows an empty frame).
const MAX_BYTES: usize = 8 * 1024 * 1024;

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
    /// URLs in flight or known-bad (no retry loop on broken screenshots).
    static PENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static FAILED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Cached screenshot (UI thread). Touches the LRU order.
pub fn get(url: &str) -> Option<Image> {
    CACHE.with(|c| c.borrow_mut().get(url))
}

/// Start loading `url` unless cached, in flight or failed before; `ready` runs
/// on the UI thread once the image is in the cache.
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

/// Decode any supported format and shrink it so the longest edge is at most `MAX_EDGE`.
pub fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some(crate::util::fit(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), w, h), MAX_EDGE))
}
