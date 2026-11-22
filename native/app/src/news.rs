//! NewsBridge: the "Novedades" drawer (Discord #noticias through the engine's
//! `fetch_news`) plus the launcher's own "Espectral Client <version>" card
//! after an update. The Slint contract lives in ui/pages/news/api.slint.
//!
//! Poll: at startup, every `POLL` minutes, on `refresh()`, and on `opened()`
//! when the last fetch is older than `OPENED_REFETCH`. Failures never drop
//! rows already on screen — `failed` only means "nothing to show".
//!
//! Seen state is prefs (`news.seen`, a Discord snowflake kept as string);
//! the update card uses `news.launcher_*`. In screenshot runs (`persist =
//! false`) nothing is written so shots keep their unread state.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::Value;
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};

use espectral_engine::news::{self as eng, NewsFeed};
use espectral_engine::Engine;

use crate::bridge::Ctx;
use crate::{i18n, prefs, util, AppWindow, NewsBridge, NewsImage as ImageRow, NewsPost as PostRow};

const POLL: Duration = Duration::from_secs(10 * 60);
/// `opened()` refetches when the last attempt is older than this.
const OPENED_REFETCH: Duration = Duration::from_secs(2 * 60);
/// The update card stays the first card this long after the update.
const CARD_WINDOW: chrono::Duration = chrono::Duration::days(7);
/// Logical width of the drawer image column; decoded images are shrunk to it.
const IMAGE_EDGE: f32 = 360.0;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

const PREF_SEEN: &str = "news.seen";
const PREF_LAUNCHER_VERSION: &str = "news.launcher_version";
const PREF_LAUNCHER_SEEN: &str = "news.launcher_seen";
const PREF_LAUNCHER_AT: &str = "news.launcher_update_at";

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Repo CHANGELOG.md (src/news.rs -> native/app/src/../../..).
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

struct PostState {
    post: eng::NewsPost,
    /// The post's `images` model — kept per row so `set_row_data` can drop a
    /// decoded image in without rebuilding the feed.
    images: Rc<VecModel<ImageRow>>,
}

impl PostState {
    fn new(post: eng::NewsPost) -> Self {
        let images = VecModel::from(
            post.images
                .iter()
                .map(|i| ImageRow {
                    image: Image::default(),
                    loaded: false,
                    width: i.width as i32,
                    height: i.height as i32,
                })
                .collect::<Vec<_>>(),
        );
        PostState { post, images: Rc::new(images) }
    }

    fn row(&self, unread: bool) -> PostRow {
        PostRow {
            id: self.post.id.as_str().into(),
            title: self.post.title.as_str().into(),
            body: self.post.body.as_str().into(),
            author: self.post.author.as_str().into(),
            created_at: self.post.created_at.as_str().into(),
            url: self.post.url.as_str().into(),
            images: ModelRc::from(self.images.clone()),
            launcher_update: false,
            unread,
        }
    }
}

/// The launcher's own card (first row) after an update.
#[derive(Clone)]
struct LauncherCard {
    id: String,
    title: String,
    body: String,
    author: String,
    created_at: String,
    unread: bool,
}

impl LauncherCard {
    fn row(&self) -> PostRow {
        PostRow {
            id: self.id.as_str().into(),
            title: self.title.as_str().into(),
            body: self.body.as_str().into(),
            author: self.author.as_str().into(),
            created_at: self.created_at.as_str().into(),
            url: SharedString::default(),
            images: ModelRc::new(VecModel::<ImageRow>::default()),
            launcher_update: true,
            unread: self.unread,
        }
    }
}

struct State {
    posts: Vec<PostState>,
    launcher: Option<LauncherCard>,
    fetching: bool,
    last_fetch: Option<Instant>,
    /// false in screenshot runs: prefs are read but never written.
    persist: bool,
    /// No `news.seen` pref yet: only the newest post counts as unread.
    first_run: bool,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    static S: RefCell<State> = const {
        RefCell::new(State { posts: Vec::new(), launcher: None, fetching: false, last_fetch: None, persist: true, first_run: false })
    };
    /// The `posts` bridge model (created once; rows are updated in place).
    static POSTS: RefCell<Option<Rc<VecModel<PostRow>>>> = const { RefCell::new(None) };
    static POLL_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
    /// Image downloads in flight (removed again on failure so a re-expand retries).
    static IMG_PENDING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn ctx() -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

fn win() -> Option<AppWindow> {
    CTX.with(|c| c.borrow().as_ref().and_then(|c| c.ui()))
}

fn posts_model() -> Option<Rc<VecModel<PostRow>>> {
    POSTS.with(|m| m.borrow().clone())
}

fn persist_set(key: &str, value: Value) {
    if with(|s| s.persist) {
        prefs::set(key, value);
    }
}

// ---------------------------------------------------------------------------
// init / fetch
// ---------------------------------------------------------------------------

pub fn init(ctx: &Ctx, ui: &AppWindow, persist: bool) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    let model: Rc<VecModel<PostRow>> = Rc::new(VecModel::default());
    POSTS.with(|m| *m.borrow_mut() = Some(model.clone()));
    let b = ui.global::<NewsBridge>();
    b.set_posts(ModelRc::from(model));
    b.set_loading(true);
    with(|s| {
        s.persist = persist;
        s.first_run = prefs::get(PREF_SEEN).is_none();
    });
    prepare_launcher_card();
    rebuild(ui);

    b.on_refresh(|| fetch(true));
    b.on_opened(opened);
    b.on_expand(|id| expand(id.to_string()));

    fetch(false);
    let t = Timer::default();
    t.start(TimerMode::Repeated, POLL, || fetch(false));
    POLL_TIMER.with(|p| *p.borrow_mut() = Some(t));
}

/// One fetch attempt; concurrent calls collapse into the one in flight.
fn fetch(force: bool) {
    let Some(c) = ctx() else { return };
    if with(|s| std::mem::replace(&mut s.fetching, true)) {
        return;
    }
    if let Some(ui) = win() {
        let b = ui.global::<NewsBridge>();
        if b.get_posts().row_count() == 0 {
            b.set_loading(true);
        }
    }
    c.call_with(
        move |e| async move { e.fetch_news(force).await },
        |ui, feed| {
            with(|s| {
                s.fetching = false;
                s.last_fetch = Some(Instant::now());
            });
            apply_feed(ui, feed);
        },
        |ui, _err| {
            with(|s| {
                s.fetching = false;
                s.last_fetch = Some(Instant::now());
            });
            let b = ui.global::<NewsBridge>();
            b.set_loading(false);
            if b.get_posts().row_count() == 0 {
                b.set_failed(true);
            }
        },
    );
}

fn apply_feed(ui: &AppWindow, feed: NewsFeed) {
    let posts = with(|s| {
        feed.posts
            .into_iter()
            .map(|p| match s.posts.iter().find(|o| o.post.id == p.id) {
                // Same post, same pictures: keep the model so decoded
                // pictures survive a refresh.
                Some(old) if same_images(&old.post.images, &p.images) => {
                    PostState { post: p, images: old.images.clone() }
                }
                _ => PostState::new(p),
            })
            .collect::<Vec<_>>()
    });
    with(|s| s.posts = posts);
    let b = ui.global::<NewsBridge>();
    b.set_channel_url(feed.channel_url.as_str().into());
    b.set_loading(false);
    b.set_failed(false);
    rebuild(ui);
    if b.get_open() {
        load_previews();
    }
}

/// Discord's image links are re-signed on every export (`?ex=…&is=…&hm=…`),
/// so pictures are the same when everything before the query matches.
fn same_images(a: &[eng::NewsImage], b: &[eng::NewsImage]) -> bool {
    let key = |i: &eng::NewsImage| (i.url.split('?').next().unwrap_or_default().to_string(), i.width, i.height);
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| key(x) == key(y))
}

// ---------------------------------------------------------------------------
// seen / unread
// ---------------------------------------------------------------------------

fn seen_id() -> u64 {
    prefs::get_str(PREF_SEEN).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Newest first: launcher card (if any) then every post.
fn rebuild(ui: &AppWindow) {
    let Some(model) = posts_model() else { return };
    let seen = seen_id();
    let rows = with(|s| {
        let first_run = s.first_run;
        let mut rows: Vec<PostRow> = Vec::new();
        if let Some(card) = &s.launcher {
            rows.push(card.row());
        }
        for (i, p) in s.posts.iter().enumerate() {
            let unread = if first_run { i == 0 } else { p.post.id.parse::<u64>().unwrap_or(0) > seen };
            rows.push(p.row(unread));
        }
        rows
    });
    model.set_vec(rows);
    sync_unread(ui);
}

/// has-unread / latest-unread-title from the live model.
fn sync_unread(ui: &AppWindow) {
    let b = ui.global::<NewsBridge>();
    let Some(model) = posts_model() else { return };
    let mut has = false;
    let mut title = SharedString::default();
    for i in 0..model.row_count() {
        if let Some(r) = model.row_data(i) {
            if r.unread {
                if !has {
                    title = r.title.clone();
                }
                has = true;
            }
        }
    }
    b.set_has_unread(has);
    b.set_latest_unread_title(title);
}

/// Drawer opened: everything on screen counts as seen.
fn opened() {
    let Some(ui) = win() else { return };
    let newest_id = with(|s| {
        if let Some(card) = s.launcher.as_mut() {
            card.unread = false;
        }
        s.first_run = false;
        s.posts.first().map(|p| p.post.id.clone()).filter(|id| !id.is_empty())
    });
    if let Some(id) = newest_id {
        persist_set(PREF_SEEN, Value::String(id));
    }
    persist_set(PREF_LAUNCHER_SEEN, Value::String(VERSION.to_string()));
    // Clear the flags in place; the rows themselves stay.
    if let Some(model) = posts_model() {
        for i in 0..model.row_count() {
            if let Some(mut row) = model.row_data(i) {
                if row.unread {
                    row.unread = false;
                    model.set_row_data(i, row);
                }
            }
        }
    }
    sync_unread(&ui);
    load_previews();
    let stale = with(|s| !s.fetching && s.last_fetch.is_none_or(|t| t.elapsed() > OPENED_REFETCH));
    if stale {
        fetch(false);
    }
}


// ---------------------------------------------------------------------------
// expand -> image loading
// ---------------------------------------------------------------------------

fn expand(id: String) {
    load_images(&id, usize::MAX);
}

/// The collapsed cards' preview: every post's first picture.
fn load_previews() {
    let ids: Vec<String> = with(|s| s.posts.iter().map(|p| p.post.id.clone()).collect());
    for id in ids {
        load_images(&id, 1);
    }
}

/// Download and decode the first `limit` pictures of post `id` that aren't
/// loaded or loading yet.
fn load_images(id: &str, limit: usize) {
    let Some(c) = ctx() else { return };
    let Some((urls, images)) =
        with(|s| s.posts.iter().find(|p| p.post.id == id).map(|p| (p.post.images.clone(), p.images.clone())))
    else {
        return;
    };
    let engine = c.engine().clone();
    for (i, img) in urls.iter().enumerate().take(limit) {
        if img.url.is_empty() || images.row_data(i).is_some_and(|r| r.loaded) {
            continue;
        }
        let url = img.url.clone();
        if !IMG_PENDING.with(|p| p.borrow_mut().insert(url.clone())) {
            continue;
        }
        let engine = engine.clone();
        let c = c.clone();
        let c_ui = c.clone();
        let post_id = id.to_string();
        c.spawn(async move {
            let bytes = fetch_bytes(&engine, &url).await;
            let buf = match bytes {
                Some(b) => tokio::task::spawn_blocking(move || decode(&b)).await.ok().flatten(),
                None => None,
            };
            c_ui.on_ui(move |_ui| {
                IMG_PENDING.with(|p| p.borrow_mut().remove(&url));
                let Some(buf) = buf else { return };
                // Look the model up again on the UI thread: the Rc can't
                // cross into the task, and a refresh may have rebuilt the row.
                let images = with(|s| s.posts.iter().find(|p| p.post.id == post_id).map(|p| p.images.clone()));
                if let Some(images) = images {
                    if let Some(mut row) = images.row_data(i) {
                        row.image = Image::from_rgba8(buf);
                        row.loaded = true;
                        images.set_row_data(i, row);
                    }
                }
            });
        });
    }
}

async fn fetch_bytes(engine: &Engine, url: &str) -> Option<Vec<u8>> {
    let resp = engine.http().client().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    (bytes.len() <= MAX_IMAGE_BYTES).then(|| bytes.to_vec())
}

/// Any format `image` reads, shrunk to the drawer's width in physical pixels
/// (2× the logical edge until the display scale is known — see icons.rs).
fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let edge = match crate::theme::phys(IMAGE_EDGE) {
        0 => (IMAGE_EDGE * 2.0) as u32,
        e => e,
    };
    Some(util::fit(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba.as_raw(), w, h), edge))
}

// ---------------------------------------------------------------------------
// launcher update card
// ---------------------------------------------------------------------------

/// The `## [<VERSION>]` section of CHANGELOG.md, flattened like a post body.
fn changelog_section(version: &str) -> Option<String> {
    let needle = format!("## [{version}]");
    let start = CHANGELOG.find(&needle)?;
    let after = &CHANGELOG[start + needle.len()..];
    // Skip the rest of the heading line (" - 2026-11-21").
    let after = after.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    let end = after.find("\n## ").unwrap_or(after.len());
    Some(eng::flatten_markdown(&after[..end]))
}

/// Decide whether the "Espectral Client <version>" card shows.
///
/// - `news.launcher_version` missing → first install: record it, no card.
/// - stored version ≠ running → an update: card unread until `opened()`
///   (`news.launcher_seen`), `news.launcher_update_at` = first sight.
/// - same version → keep showing the card while `update_at` is < 7 days old.
/// The running version is always written back to `news.launcher_version`.
fn prepare_launcher_card() {
    let now = Utc::now();
    let prev = prefs::get_str(PREF_LAUNCHER_VERSION);
    let seen = prefs::get_str(PREF_LAUNCHER_SEEN);
    let stored_at = prefs::get_str(PREF_LAUNCHER_AT).and_then(|a| {
        DateTime::parse_from_rfc3339(&a).ok().map(|t| (a, t.with_timezone(&Utc)))
    });

    let card_at: Option<String> = match prev.as_deref() {
        None => None, // first install: record, no card
        Some(v) if v != VERSION => {
            let at = now.to_rfc3339();
            persist_set(PREF_LAUNCHER_AT, Value::String(at.clone()));
            Some(at)
        }
        Some(_) => stored_at.and_then(|(a, t)| (now.signed_duration_since(t) <= CARD_WINDOW).then_some(a)),
    };

    if let Some(at) = card_at {
        with(|s| {
            s.launcher = Some(LauncherCard {
                id: format!("launcher-{VERSION}"),
                title: i18n::tf("news.updateTitle", &[("version", VERSION)]),
                author: i18n::t("news.updateAuthor"),
                created_at: at,
                body: changelog_section(VERSION).unwrap_or_default(),
                unread: seen.as_deref() != Some(VERSION),
            });
        });
    }
    persist_set(PREF_LAUNCHER_VERSION, Value::String(VERSION.to_string()));
}
