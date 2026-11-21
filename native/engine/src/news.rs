//! The "Novedades" feed: #noticias posts exported by the BalasGPT bot at
//! `ContentSettings::news_url` (default `NEWS_API`, `ESPECTRAL_NEWS_URL` wins
//! over everything — point it at a local test feed). Owner: content.
//!
//! The wire JSON (`{ version, channel_url, fetched_at, messages[] }`) keeps
//! Discord's newest-first order and one entry per message; readers see posts:
//! consecutive messages of the same author within `GROUP_WINDOW` merge into
//! one post whose id/url/time come from the group's FIRST (oldest) message.
//!
//! On success the raw payload lands in `<data>/news-cache.json`; a failed
//! fetch falls back to it with `stale: true`, and a failure with no cache is
//! an error. `force` only skips the `MEMO_TTL` in-process copy.
//!
//! `file://` URLs read a local JSON document (dev/test feeds, e.g.
//! `file:///C:/tmp/news.json`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::download::FetchOptions;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult};

/// BalasGPT export of the #noticias channel.
pub const NEWS_API: &str = "https://espectral.uhce.top/config/api/news";
/// Env var that overrides `ContentSettings::news_url` (local test feeds).
pub const NEWS_URL_ENV: &str = "ESPECTRAL_NEWS_URL";
/// The feed is chat, not an API: keep the request short.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Same-author messages this close together are one post.
const GROUP_WINDOW: chrono::Duration = chrono::Duration::minutes(15);
/// Re-fetches inside this window reuse the in-memory copy (`force` skips it).
const MEMO_TTL: Duration = Duration::from_secs(60);
const CACHE_FILE: &str = "news-cache.json";

// ---------------------------------------------------------------------------
// Public shape
// ---------------------------------------------------------------------------

/// One picture of a post (attachment or embed image).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsImage {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

/// A post: one author burst, titled by its heading or first paragraph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsPost {
    /// Discord snowflake of the group's first message ("" never emitted).
    pub id: String,
    /// First `# ` heading of the text, else its first paragraph ("" = none).
    pub title: String,
    /// The rest of the text, light markdown flattened to plain text.
    pub body: String,
    pub author: String,
    /// RFC 3339 of the group's first message.
    pub created_at: String,
    /// Discord link of the group's first message ("" when absent).
    pub url: String,
    pub images: Vec<NewsImage>,
}

/// The shaped feed the app shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsFeed {
    /// Newest first.
    pub posts: Vec<NewsPost>,
    pub channel_url: String,
    /// RFC 3339 of the payload (or of the fetch when the feed omits it).
    pub fetched_at: String,
    /// Served from `<data>/news-cache.json` because the fetch failed.
    pub stale: bool,
}

// ---------------------------------------------------------------------------
// Wire shape (tolerant: unknown fields ignored, everything optional)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct WireEmbed {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    images: Vec<NewsImage>,
    #[serde(default)]
    embeds: Vec<WireEmbed>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireFeed {
    #[serde(default)]
    channel_url: Option<String>,
    #[serde(default)]
    fetched_at: Option<String>,
    #[serde(default)]
    messages: Vec<WireMessage>,
}

// ---------------------------------------------------------------------------
// Fetch + cache
// ---------------------------------------------------------------------------

fn cache_path(engine: &Engine) -> PathBuf {
    engine.paths().data_dir().join(CACHE_FILE)
}

fn feed_url(engine: &Engine) -> String {
    if let Ok(u) = std::env::var(NEWS_URL_ENV) {
        if !u.trim().is_empty() {
            return u.trim().to_string();
        }
    }
    engine.content_settings().news_url.trim().to_string()
}

async fn fetch_wire(engine: &Engine, url: &str) -> EngineResult<WireFeed> {
    if let Some(rest) = url.strip_prefix("file://") {
        let path = url::Url::parse(url)
            .ok()
            .and_then(|u| u.to_file_path().ok())
            .unwrap_or_else(|| PathBuf::from(rest));
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| EngineError::other(format!("news feed {url}: {e}")))?;
        return serde_json::from_slice(&bytes)
            .map_err(|e| EngineError::other(format!("news feed {url}: invalid JSON: {e}")));
    }
    engine
        .http()
        .fetch_json(url, &FetchOptions { headers: vec![], timeout: FETCH_TIMEOUT })
        .await
}

async fn read_cache(engine: &Engine) -> Option<WireFeed> {
    let path = cache_path(engine);
    let bytes = tokio::fs::read(&path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// GET the feed and shape it into posts. Falls back to `<data>/news-cache.json`
/// (marked `stale`) when the fetch fails; `force` skips the short in-memory
/// copy so a manual refresh really reaches the network.
pub async fn fetch_news(engine: &Engine, force: bool) -> EngineResult<NewsFeed> {
    if !force {
        let memo = engine.content_state().news_memo.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, feed)) = memo.as_ref() {
            if at.elapsed() < MEMO_TTL {
                return Ok(feed.clone());
            }
        }
    }
    let url = feed_url(engine);
    match fetch_wire(engine, &url).await {
        Ok(wire) => {
            if let Err(e) = crate::paths::write_json_atomic(&cache_path(engine), &wire) {
                tracing::warn!("[news] could not write {}: {e}", cache_path(engine).display());
            }
            let feed = shape(wire, false);
            *engine.content_state().news_memo.lock().unwrap_or_else(|p| p.into_inner()) =
                Some((Instant::now(), feed.clone()));
            Ok(feed)
        }
        Err(e) => match read_cache(engine).await {
            Some(wire) => {
                tracing::warn!("[news] fetch failed ({e}); serving cache");
                Ok(shape(wire, true))
            }
            None => Err(e),
        },
    }
}

// ---------------------------------------------------------------------------
// Shaping (pure)
// ---------------------------------------------------------------------------

/// Accumulates one post while walking messages oldest -> newest.
struct PendingPost {
    id: String,
    url: String,
    author: String,
    created_at: String,
    first_time: Option<DateTime<Utc>>,
    texts: Vec<String>,
    images: Vec<NewsImage>,
    embeds: Vec<WireEmbed>,
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

fn shape(wire: WireFeed, stale: bool) -> NewsFeed {
    let mut posts: Vec<NewsPost> = Vec::new();
    let mut pending: Option<PendingPost> = None;
    // Wire order is newest-first; groups form in chronological order.
    for msg in wire.messages.into_iter().rev() {
        let author = msg.author.unwrap_or_default();
        let created = msg.created_at.unwrap_or_default();
        let time = parse_time(&created);
        let merges = pending.as_ref().is_some_and(|p| {
            p.author == author && p.first_time.zip(time).is_some_and(|(a, b)| (b - a).abs() <= GROUP_WINDOW)
        });
        if !merges {
            if let Some(p) = pending.take() {
                posts.push(finish(p));
            }
            pending = Some(PendingPost {
                id: msg.id.unwrap_or_default(),
                url: msg.url.unwrap_or_default(),
                author,
                created_at: created,
                first_time: time,
                texts: Vec::new(),
                images: Vec::new(),
                embeds: Vec::new(),
            });
        }
        if let Some(p) = pending.as_mut() {
            let text = msg.content.unwrap_or_default();
            if !text.trim().is_empty() {
                p.texts.push(text);
            }
            p.images.extend(msg.images);
            p.embeds.extend(msg.embeds);
        }
    }
    if let Some(p) = pending.take() {
        posts.push(finish(p));
    }
    // Restore newest-first.
    posts.reverse();
    NewsFeed {
        posts,
        channel_url: wire.channel_url.unwrap_or_default(),
        fetched_at: wire.fetched_at.unwrap_or_else(|| Utc::now().to_rfc3339()),
        stale,
    }
}

fn finish(p: PendingPost) -> NewsPost {
    let content = p.texts.join("\n\n");
    let (title, body) = split_title_body(&content);
    // Empty or URL-only content: the embed IS the post (tweet cards, YouTube
    // links). Its title wins; a title-less card (X/Twitter) takes its title
    // from the description's first paragraph instead of showing the URL.
    if content_is_only_url_or_empty(&content) {
        let titled = p.embeds.iter().find(|e| e.title.as_deref().is_some_and(|t| !t.trim().is_empty()));
        let described = p.embeds.iter().find(|e| e.description.as_deref().is_some_and(|d| !d.trim().is_empty()));
        let embed_post = match (titled, described) {
            (Some(e), _) => Some((clean_title(e.title.as_deref().unwrap_or_default()), flatten_markdown(e.description.as_deref().unwrap_or_default()))),
            (None, Some(e)) => Some(split_title_body(e.description.as_deref().unwrap_or_default())),
            (None, None) => None,
        };
        if let Some((title, body)) = embed_post {
            return NewsPost { id: p.id, title, body, author: p.author, created_at: p.created_at, url: p.url, images: p.images };
        }
    }
    NewsPost { id: p.id, title, body, author: p.author, created_at: p.created_at, url: p.url, images: p.images }
}

/// True when the text holds no post of its own: empty, or one bare URL.
fn content_is_only_url_or_empty(content: &str) -> bool {
    let t = content.trim();
    t.is_empty()
        || (t.starts_with("http://") || t.starts_with("https://")) && !t.chars().any(char::is_whitespace)
}

/// `#`/`##`/`###` + space (user's rule); the text after it, or None.
fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=3).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    rest.strip_prefix(' ').map(str::trim)
}

/// Whitespace-collapsed title minus leftover decoration noise at the edges.
/// `!`, `?`, `¡`, `¿`, `)` survive — they end sentences, not markdown.
fn clean_title(raw: &str) -> String {
    const NOISE: &[char] = &[' ', '\t', '*', '_', '~', '`', '>', '#', '-', '—', '–', '…', '•', '·', '.', ',', ':', ';'];
    strip_inline(raw).split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(NOISE).to_string()
}

fn is_noise(paragraph: &str) -> bool {
    clean_title(paragraph).is_empty()
}

/// The title rule: first markdown heading anywhere wins (and leaves the body);
/// else the first paragraph does. Punctuation-only paragraphs are noise, not
/// a title.
fn split_title_body(content: &str) -> (String, String) {
    // Heading rule: scan line by line.
    let mut lines = content.split('\n').collect::<Vec<_>>();
    for (i, line) in lines.iter().enumerate() {
        let Some(text) = heading_text(line.trim_start()) else { continue };
        let title = clean_title(text);
        if title.is_empty() {
            continue; // "###" alone is decoration; drop it and keep looking.
        }
        lines.remove(i);
        return (title, flatten_markdown(&lines.join("\n")));
    }
    // First-paragraph rule: paragraphs are blank-line separated.
    let mut paragraphs = content.split("\n\n").collect::<Vec<_>>();
    for (i, para) in paragraphs.iter().enumerate() {
        let title = clean_title(para);
        if title.is_empty() {
            continue; // leading noise/blank chunks are not the title.
        }
        paragraphs.remove(i);
        return (title, flatten_markdown(&paragraphs.join("\n\n")));
    }
    (String::new(), flatten_markdown(content))
}

fn unpair(s: &str, marker: &str) -> String {
    s.replace(marker, "")
}

/// Strip `*...*` pairs (paired only: an opener needs a non-space right after,
/// a closer a non-space right before — `* * *` dividers and stray `*` stay).
fn unpair_italics(s: &str) -> String {
    let b = s.as_bytes();
    let mut keep = vec![true; b.len()];
    let mut open: Option<usize> = None;
    for (i, &c) in b.iter().enumerate() {
        if c != b'*' {
            continue;
        }
        match open {
            None => {
                if b.get(i + 1).is_some_and(|n| !n.is_ascii_whitespace()) {
                    open = Some(i);
                }
            }
            Some(o) => {
                if i > 0 && !b[i - 1].is_ascii_whitespace() {
                    keep[o] = false;
                    keep[i] = false;
                    open = None;
                }
            }
        }
    }
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        if keep[i] {
            out.push(c);
        }
    }
    out
}

fn strip_inline(s: &str) -> String {
    let s = strip_links(s);
    let s = unpair(&s, "**");
    let s = unpair(&s, "__");
    let s = unpair_italics(&s);
    let s = unpair(&s, "~~");
    unescape(&unpair(&s, "`"))
}

/// Markdown backslash escapes (`1\.7`, `\-`, `\_` in tweet cards) -> the
/// character itself.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            continue;
        }
        out.push(c);
    }
    out
}

/// `[text](url)` -> `text` (unbalanced brackets are kept as-is).
fn strip_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < s.len() {
        if bytes[i] == b'[' {
            if let Some(close) = s[i..].find(']').map(|d| i + d) {
                if bytes.get(close + 1) == Some(&b'(') {
                    if let Some(end) = s[close + 1..].find(')').map(|d| close + 1 + d) {
                        out.push_str(&s[i + 1..close]);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        let end = i + s[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&s[i..end]);
        i = end;
    }
    out
}

/// Light markdown -> plain text (news bodies AND the changelog's version
/// section): headings become plain lines, `- `/`* ` items become `• `,
/// `> ` quotes lose their marker, `[text](url)` keeps `text`, emphasis /
/// strike / code markers are removed, blank runs collapse to one blank line,
/// punctuation-only edge paragraphs are dropped.
pub fn flatten_markdown(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut blank = true; // leading blanks produce nothing
    for line in markdown.split('\n') {
        if line.trim().is_empty() {
            blank = true;
            continue;
        }
        if !out.is_empty() && blank {
            out.push(String::new());
        }
        blank = false;
        let mut t = line.trim().to_string();
        while let Some(rest) = t.strip_prefix('>') {
            t = rest.trim_start().to_string();
        }
        if let Some(h) = heading_text(&t) {
            t = h.to_string();
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            t = format!("• {}", item.trim_start());
        }
        out.push(strip_inline(&t));
    }
    // Drop noise paragraphs off the edges ("---", stray "__", …).
    let joined = out.join("\n");
    let mut paras: Vec<&str> = joined.split("\n\n").collect();
    while paras.first().is_some_and(|p| is_noise(p)) {
        paras.remove(0);
    }
    while paras.last().is_some_and(|p| is_noise(p)) {
        paras.pop();
    }
    paras.join("\n\n").trim().to_string()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use serde_json::json;

    fn engine_with(server: Option<&MockServer>) -> (tempfile::TempDir, Engine) {
        let (d, e) = temp_engine();
        let url = server.map(|s| s.url("/api/news")).unwrap_or_else(|| "http://127.0.0.1:9/api/news".into());
        e.update_content_settings(|s| s.news_url = url);
        (d, e)
    }

    fn msg(id: &str, author: &str, at: &str, content: &str) -> serde_json::Value {
        json!({ "id": id, "author": author, "created_at": at, "content": content })
    }

    fn feed(messages: serde_json::Value) -> serde_json::Value {
        json!({
            "version": 1,
            "channel_url": "https://discord.com/channels/1/2",
            "fetched_at": "2026-11-21T14:00:00+00:00",
            "messages": messages,
        })
    }

    #[test]
    fn title_from_heading() {
        let (title, body) =
            split_title_body("# 🖌️ Resource pack (opcional) para UHCs!\nA partir de ahora en main.uhce.top");
        assert_eq!(title, "🖌️ Resource pack (opcional) para UHCs!");
        assert_eq!(body, "A partir de ahora en main.uhce.top");
    }

    #[test]
    fn title_from_first_paragraph() {
        let (title, body) = split_title_body("Ping medio en Espectral en los ultimos 11 dias\n\nDetalles aqui");
        assert_eq!(title, "Ping medio en Espectral en los ultimos 11 dias");
        assert_eq!(body, "Detalles aqui");
        // A punctuation-only first paragraph is noise, not the title.
        let (title, body) = split_title_body("---\n\nReal title\n\nbody text");
        assert_eq!(title, "Real title");
        assert_eq!(body, "body text");
    }

    #[tokio::test]
    async fn groups_same_author_burst() {
        // The real channel pair: a heading post, then a follow-up line by the
        // same author 44 s later — newest first on the wire.
        let server = MockServer::start(move |_| {
            MockResponse::json(&feed(json!([
                msg("2002", "Hen", "2026-11-21T13:18:47.337Z", "La screenshot es antes del cambio"),
                msg("2001", "Hen", "2026-11-21T13:18:03.777Z", "# 🖌️ Resource pack\nA partir de ahora"),
                msg("1999", "Hen", "2026-11-20T09:28:41.910Z", "Ping medio en Espectral"),
            ])))
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let feed = fetch_news(&e, false).await.unwrap();
        assert_eq!(feed.posts.len(), 2);
        let burst = &feed.posts[0];
        // Group id/url/time come from the FIRST (oldest) message of the group.
        assert_eq!(burst.id, "2001");
        assert_eq!(burst.created_at, "2026-11-21T13:18:03.777Z");
        assert_eq!(burst.title, "🖌️ Resource pack");
        assert_eq!(burst.body, "A partir de ahora\n\nLa screenshot es antes del cambio");
        assert_eq!(feed.posts[1].id, "1999");
        assert_eq!(feed.channel_url, "https://discord.com/channels/1/2");
    }

    #[tokio::test]
    async fn link_only_post_uses_embed() {
        let server = MockServer::start(move |_| {
            MockResponse::json(&feed(json!([{
                "id": "3001", "author": "Hen", "created_at": "2026-11-15T02:17:53.362Z",
                "url": "https://discord.com/channels/1/2/3001",
                "content": "https://youtube.com/shorts/VAT08tTntrA",
                "embeds": [{ "url": "https://youtube.com/shorts/VAT08tTntrA",
                             "title": "UHC Espectral? #uhc", "description": "clip desc", "provider": "YouTube" }]
            }])))
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let feed = fetch_news(&e, false).await.unwrap();
        assert_eq!(feed.posts.len(), 1);
        assert_eq!(feed.posts[0].title, "UHC Espectral? #uhc");
        assert_eq!(feed.posts[0].body, "clip desc");
        assert_eq!(feed.posts[0].url, "https://discord.com/channels/1/2/3001");
    }

    /// X/Twitter cards have no title: the tweet's first paragraph is the
    /// title (escapes removed), not the bare link.
    #[tokio::test]
    async fn titleless_card_uses_its_description() {
        let server = MockServer::start(move |_| {
            MockResponse::json(&feed(json!([{
                "id": "3101", "author": "Hen", "created_at": "2026-11-10T10:00:00Z",
                "url": "https://discord.com/channels/1/2/3101",
                "content": "https://x.com/GamesEspectral/status/1",
                "embeds": [{ "url": "https://twitter.com/GamesEspectral/status/1", "title": null,
                             "description": "⚛️Espectral Client 1\\.7\nDesde la última actualización;\n\n\\-⚙️Rewrite en Rust" }]
            }])))
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let post = &fetch_news(&e, false).await.unwrap().posts[0];
        assert_eq!(post.title, "⚛️Espectral Client 1.7 Desde la última actualización");
        assert_eq!(post.body, "-⚙️Rewrite en Rust");
    }

    #[tokio::test]
    async fn failure_falls_back_to_cache() {
        // First fetch succeeds and writes the cache…
        let good = MockServer::start(move |_| {
            MockResponse::json(&feed(json!([msg("4001", "Hen", "2026-11-21T10:00:00Z", "# Hola\nMundo")])))
        })
        .await;
        let (d, e) = engine_with(Some(&good));
        let fresh = fetch_news(&e, false).await.unwrap();
        assert!(!fresh.stale);
        drop(good);
        assert!(d.path().join(CACHE_FILE).exists());
        // …then the endpoint dies: force past the memo, cache serves stale.
        e.update_content_settings(|s| s.news_url = "http://127.0.0.1:9/api/news".into());
        let stale = fetch_news(&e, true).await.unwrap();
        assert!(stale.stale);
        assert_eq!(stale.posts, fresh.posts);
        // No cache at all + failure is an error.
        let (_d2, e2) = temp_engine();
        e2.update_content_settings(|s| s.news_url = "http://127.0.0.1:9/api/news".into());
        assert!(fetch_news(&e2, true).await.is_err());
    }

    #[tokio::test]
    async fn failure_500_falls_back_to_cache() {
        // Sequence: 200 with a post, then 500 — served stale from disk.
        let server = MockServer::start_seq(move |n, _| {
            if n == 0 {
                MockResponse::json(&feed(json!([msg("4002", "Hen", "2026-11-21T10:00:00Z", "# Hola\nMundo")])))
            } else {
                MockResponse::status(500)
            }
        })
        .await;
        let (_d, e) = engine_with(Some(&server));
        let fresh = fetch_news(&e, false).await.unwrap();
        let stale = fetch_news(&e, true).await.unwrap();
        assert!(stale.stale);
        assert_eq!(stale.posts, fresh.posts);
        assert_eq!(server.hits(), 2);
    }

    #[test]
    fn flattens_markdown() {
        let flat = flatten_markdown(
            "### Fixed\n- **Bold** thing\n* it2\n> quoted\n[link](https://x.test) and `code` and ~~gone~~\n\n\n\nnext para\nhttps://whole-line.test/",
        );
        assert_eq!(
            flat,
            "Fixed\n• Bold thing\n• it2\nquoted\nlink and code and gone\n\nnext para\nhttps://whole-line.test/"
        );
    }
}
