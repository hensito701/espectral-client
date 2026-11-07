//! DB-IP "IP to Country Lite" → compact `ESPGEO1` country table for the
//! in-game multiplayer list (server-row country flags).
//!
//! Data: <https://db-ip.com> — "IP Geolocation by DB-IP" (CC BY 4.0), the
//! monthly `dbip-country-lite-YYYY-MM.csv.gz` (headerless `start,end,CC`
//! rows). The launcher downloads it in the background at start, converts it
//! to a fixed-width binary table and passes the absolute path to the game as
//! `-Despectral.geoip=<path>`; the in-game mod memory-maps it on a daemon
//! thread and never touches the file again. Every step (download, gunzip,
//! parse, encode, write) can fail without affecting a launch: the property is
//! only added when [`table_path`] validates a table, so a failed refresh for
//! this install simply means no flags.
//!
//! ## `ESPGEO1` file layout (little-endian, no padding)
//!
//! ```text
//! offset  size               field
//! 0       8                  magic  b"ESPGEO1\0"
//! 8       4                  u32 v4_count      (IPv4 entries)
//! 12      4                  u32 v6_count      (IPv6 entries)
//! 16      4                  u32 country_count (2-byte codes)
//! 20      4                  u32 reserved (must be 0)
//! 24      2 * country_count  ISO-3166 alpha-2 codes, ASCII, ascending
//! 24+2C   34 * v4_count      IPv4 entries, ascending start, no overlap
//! …       34 * v6_count      IPv6 entries, ascending start, no overlap
//! ```
//!
//! One entry is `start[16] end[16] u16 country_index` (little-endian index):
//! addresses are 16-byte **big-endian** so one lexicographic compare is the
//! numeric compare, and IPv4 is stored IPv4-mapped (`::ffff:a.b.c.d`) so the
//! same shape works for both families. `ZZ` (DB-IP's "unknown") is dropped at
//! conversion time; adjacent ranges of the same country are merged.
//!
//! ## Lookup (reference for the Java reader)
//!
//! [`lookup`] is the reference implementation:
//!
//! 1. key = the address octets; an IPv6 `::ffff:a.b.c.d` (or a plain IPv4)
//!    uses the IPv4 section, everything else the IPv6 section;
//! 2. `i = partition_point(start <= key)` (i.e. upper bound of starts);
//! 3. if `i > 0` and `key <= entries[i-1].end`, the country is
//!    `countries[entries[i-1].country]`, else `null`.
//!
//! DB-IP's releases are disjoint, so step 2 always finds the one range that
//! can cover the address. Should a release ever overlap (it has not), the
//! later-starting range wins, which is what the upper bound above returns.
//!
//! Anycast / DDoS-protected servers resolve to their proxy's country: the
//! table answers "where is this IP announced", not "where is the operator".
//!
//! Refresh policy ([`refresh`]): reuse the table while `meta.json` names the
//! current month and the file is younger than 35 days; otherwise download the
//! current month (falling back to the previous one while the new one is not
//! published yet), rebuild, write the table, then `meta.json`, then delete the
//! older tables. A failed refresh leaves the previous table in place.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Months, Utc};
use serde::{Deserialize, Serialize};

use crate::download::FetchOptions;
use crate::engine::Engine;
use crate::error::{EngineError, EngineResult, HttpErrorKind};
use crate::paths::{self, Paths};

/// DB-IP monthly downloads base (append `/dbip-country-lite-<YYYY-MM>.csv.gz`).
pub const DBIP_BASE: &str = "https://download.db-ip.com/free";
/// Mandatory DB-IP attribution (CC BY 4.0); the mod shows it in the flag tooltip.
pub const ATTRIBUTION: &str = "IP Geolocation by DB-IP";
/// DB-IP site for the attribution link.
pub const ATTRIBUTION_URL: &str = "https://db-ip.com";
/// File magic, NUL-terminated so it is exactly 8 bytes.
pub const MAGIC: &[u8; 8] = b"ESPGEO1\0";
/// `magic + v4 + v6 + countries + reserved`.
pub const HEADER_LEN: usize = 24;
/// `start[16] + end[16] + country index u16`.
pub const ENTRY_LEN: usize = 34;
/// Reuse window for an already downloaded table (design: `< 35 days`).
const MAX_AGE_SECS: i64 = 35 * 24 * 60 * 60;
/// Decompressed CSV cap — the real file is ~40 MB; anything far bigger is not
/// the file we asked for (a gzip bomb would otherwise fill memory).
const MAX_CSV_BYTES: u64 = 512 * 1024 * 1024;

/// One range: big-endian 16-byte start/end plus a country-table index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeoRange {
    pub start: [u8; 16],
    pub end: [u8; 16],
    pub country: u16,
}

/// A parsed country table, exactly what [`encode`] writes / [`decode`] reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeoTable {
    /// ISO-3166 alpha-2 codes, ascending; `GeoRange::country` indexes this.
    pub countries: Vec<[u8; 2]>,
    /// IPv4 ranges (IPv4-mapped encoding), ascending start, non-overlapping.
    pub v4: Vec<GeoRange>,
    /// IPv6 ranges, ascending start, non-overlapping.
    pub v6: Vec<GeoRange>,
}

/// `meta.json`: which table is current and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoMeta {
    /// `YYYY-MM` of the DB-IP release.
    pub month: String,
    /// File name inside `cache/geoip/` (never a path).
    pub file: String,
    /// Source URL the table was built from.
    #[serde(default)]
    pub source: String,
    /// RFC 3339 UTC build time ("" when an older/hand-written meta lacks it).
    #[serde(default)]
    pub refreshed_at: String,
    #[serde(default)]
    pub v4_ranges: u64,
    #[serde(default)]
    pub v6_ranges: u64,
    #[serde(default)]
    pub countries: u64,
    #[serde(default)]
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

impl GeoTable {
    /// Entries in the encoded order: IPv4 first, then IPv6.
    fn entry_count(&self) -> usize {
        self.v4.len() + self.v6.len()
    }
}

/// Serialize a table to `ESPGEO1` bytes.
pub fn encode(table: &GeoTable) -> Result<Vec<u8>, String> {
    if table.countries.len() > u16::MAX as usize {
        return Err(format!("too many countries ({})", table.countries.len()));
    }
    let v4_count = u32::try_from(table.v4.len()).map_err(|_| "too many IPv4 ranges".to_string())?;
    let v6_count = u32::try_from(table.v6.len()).map_err(|_| "too many IPv6 ranges".to_string())?;
    let mut out =
        Vec::with_capacity(HEADER_LEN + table.countries.len() * 2 + table.entry_count() * ENTRY_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&v4_count.to_le_bytes());
    out.extend_from_slice(&v6_count.to_le_bytes());
    out.extend_from_slice(&(table.countries.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for cc in &table.countries {
        out.extend_from_slice(cc);
    }
    for r in table.v4.iter().chain(table.v6.iter()) {
        out.extend_from_slice(&r.start);
        out.extend_from_slice(&r.end);
        out.extend_from_slice(&r.country.to_le_bytes());
    }
    Ok(out)
}

/// Parsed header: entry/country counts plus the total file size they imply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    v4: u32,
    v6: u32,
    countries: u32,
}

impl Header {
    fn total_len(&self) -> u64 {
        let entries = u64::from(self.v4) + u64::from(self.v6);
        (HEADER_LEN + self.countries as usize * 2) as u64 + entries * ENTRY_LEN as u64
    }
}

fn parse_header(bytes: &[u8]) -> Result<Header, String> {
    if bytes.len() < HEADER_LEN {
        return Err(format!("truncated header ({} bytes)", bytes.len()));
    }
    if &bytes[..8] != MAGIC {
        return Err("bad magic (not an ESPGEO1 table)".into());
    }
    let word = |at: usize| {
        let mut w = [0u8; 4];
        w.copy_from_slice(&bytes[at..at + 4]);
        u32::from_le_bytes(w)
    };
    let (v4, v6, countries, reserved) = (word(8), word(12), word(16), word(20));
    if reserved != 0 {
        return Err("unsupported table version (reserved word is not 0)".into());
    }
    Ok(Header { v4, v6, countries })
}

/// Read a table back (used by the round-trip test; also the on-disk spec).
pub fn decode(bytes: &[u8]) -> Result<GeoTable, String> {
    let head = parse_header(bytes)?;
    let expected = head.total_len();
    if bytes.len() as u64 != expected {
        return Err(format!("bad length: {} bytes, header says {expected}", bytes.len()));
    }
    let mut at = HEADER_LEN;
    let mut countries = Vec::with_capacity(head.countries as usize);
    for _ in 0..head.countries {
        countries.push([bytes[at], bytes[at + 1]]);
        at += 2;
    }
    let mut read_ranges = |count: u32| -> Vec<GeoRange> {
        let mut v = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut r = GeoRange { start: [0; 16], end: [0; 16], country: 0 };
            r.start.copy_from_slice(&bytes[at..at + 16]);
            r.end.copy_from_slice(&bytes[at + 16..at + 32]);
            let mut w = [0u8; 2];
            w.copy_from_slice(&bytes[at + 32..at + 34]);
            r.country = u16::from_le_bytes(w);
            v.push(r);
            at += ENTRY_LEN;
        }
        v
    };
    let v4 = read_ranges(head.v4);
    let v6 = read_ranges(head.v6);
    Ok(GeoTable { countries, v4, v6 })
}

// ---------------------------------------------------------------------------
// CSV -> table
// ---------------------------------------------------------------------------

/// A CSV row before country dedup (the code is resolved to an index later).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawRange {
    start: [u8; 16],
    end: [u8; 16],
    cc: [u8; 2],
}

/// IPv4 octets as the IPv4-mapped IPv6 form (`::ffff:a.b.c.d`).
fn mapped(addr: Ipv4Addr) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[10] = 0xff;
    out[11] = 0xff;
    out[12..].copy_from_slice(&addr.octets());
    out
}

/// Big-endian + 1, `None` at `ff…ff`.
fn increment(v: &[u8; 16]) -> Option<[u8; 16]> {
    let mut out = *v;
    for i in (0..16).rev() {
        if out[i] == 0xff {
            out[i] = 0;
        } else {
            out[i] += 1;
            return Some(out);
        }
    }
    None
}

/// `next_start <= end + 1` (adjacent or overlapping).
fn touches(end: &[u8; 16], next_start: &[u8; 16]) -> bool {
    match increment(end) {
        Some(e1) => *next_start <= e1,
        None => true,
    }
}

/// Sort by start and merge adjacent/overlapping ranges of the same country.
fn merge(mut rows: Vec<RawRange>) -> Vec<RawRange> {
    rows.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let mut out: Vec<RawRange> = Vec::with_capacity(rows.len());
    for r in rows {
        match out.last_mut() {
            Some(prev) if prev.cc == r.cc && touches(&prev.end, &r.start) => {
                if r.end > prev.end {
                    prev.end = r.end;
                }
            }
            _ => out.push(r),
        }
    }
    out
}

/// `(row, is_ipv4_section)`; `None` for a row that is not `addr,addr,CC`.
fn parse_row(start: &str, end: &str, cc: &str) -> Option<(RawRange, bool)> {
    let (s, e, v4) = if start.contains(':') {
        let s: Ipv6Addr = start.parse().ok()?;
        let e: Ipv6Addr = end.parse().ok()?;
        match (s.to_ipv4_mapped(), e.to_ipv4_mapped()) {
            (Some(s4), Some(e4)) => (mapped(s4), mapped(e4), true),
            _ => (s.octets(), e.octets(), false),
        }
    } else {
        let s: Ipv4Addr = start.parse().ok()?;
        let e: Ipv4Addr = end.parse().ok()?;
        (mapped(s), mapped(e), true)
    };
    if e < s {
        return None;
    }
    let code = cc.trim().as_bytes();
    if code.len() != 2 || !code.iter().all(u8::is_ascii_alphabetic) {
        return None;
    }
    Some((
        RawRange { start: s, end: e, cc: [code[0].to_ascii_uppercase(), code[1].to_ascii_uppercase()] },
        v4,
    ))
}

/// Parse DB-IP's headerless `start,end,CC` CSV. Malformed rows and `ZZ`
/// (unknown) ranges are skipped; the result is sorted and merged.
pub fn parse_csv(csv: &str) -> GeoTable {
    let mut v4: Vec<RawRange> = Vec::new();
    let mut v6: Vec<RawRange> = Vec::new();
    let mut skipped = 0usize;
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.splitn(3, ',');
        let (Some(start), Some(end), Some(cc)) = (fields.next(), fields.next(), fields.next()) else {
            skipped += 1;
            continue;
        };
        if cc.trim().eq_ignore_ascii_case("ZZ") {
            continue; // "unknown" — no flag rather than a wrong one
        }
        match parse_row(start.trim(), end.trim(), cc) {
            Some((row, true)) => v4.push(row),
            Some((row, false)) => v6.push(row),
            None => skipped += 1,
        }
    }
    if skipped > 0 {
        tracing::debug!("[geoip] skipped {skipped} malformed CSV rows");
    }
    let v4 = merge(v4);
    let v6 = merge(v6);

    let mut codes: Vec<[u8; 2]> = v4.iter().chain(v6.iter()).map(|r| r.cc).collect();
    codes.sort_unstable();
    codes.dedup();
    fn to_ranges(rows: &[RawRange], codes: &[[u8; 2]]) -> Vec<GeoRange> {
        rows.iter()
            .map(|r| GeoRange {
                start: r.start,
                end: r.end,
                country: codes.binary_search(&r.cc).unwrap_or_default() as u16,
            })
            .collect()
    }
    let (v4, v6) = (to_ranges(&v4, &codes), to_ranges(&v6, &codes));
    GeoTable { countries: codes, v4, v6 }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// Country (`ISO-3166 alpha-2`) covering `ip`, or `None` in a gap.
///
/// Reference implementation for the mod's Java reader: see the module docs.
pub fn lookup(table: &GeoTable, ip: IpAddr) -> Option<String> {
    let (key, section) = match ip {
        IpAddr::V4(v4) => (mapped(v4), &table.v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => (mapped(v4), &table.v4),
            None => (v6.octets(), &table.v6),
        },
    };
    let i = section.partition_point(|r| r.start <= key);
    let r = i.checked_sub(1).and_then(|i| section.get(i))?;
    if key <= r.end {
        let cc = table.countries.get(r.country as usize)?;
        return Some(String::from_utf8_lossy(cc).into_owned());
    }
    None
}

// ---------------------------------------------------------------------------
// Paths / current table
// ---------------------------------------------------------------------------

fn valid_table_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("countries-") else { return false };
    let Some(month) = rest.strip_suffix(".bin") else { return false };
    is_month(month)
}

fn is_month(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && (1..=12).contains(&(u32::from(b[5] - b'0') * 10 + u32::from(b[6] - b'0')))
}

fn month_key(at: DateTime<Utc>) -> String {
    at.format("%Y-%m").to_string()
}

/// The table `meta.json` points at, validated (magic, reserved, exact size).
///
/// `None` when there is no meta, the pointer is not a plain file name, the
/// file is missing or its header disagrees with its size — in every one of
/// those cases a launch must not advertise a table it cannot trust.
pub fn table_path(paths: &Paths) -> Option<PathBuf> {
    let meta: GeoMeta = serde_json::from_str(&std::fs::read_to_string(paths.geoip_meta()).ok()?).ok()?;
    if !valid_table_name(&meta.file) {
        return None;
    }
    let path = paths.geoip_dir().join(&meta.file);
    let size = std::fs::metadata(&path).ok()?.len();
    let mut head = [0u8; HEADER_LEN];
    std::fs::File::open(&path).ok()?.read_exact(&mut head).ok()?;
    let parsed = parse_header(&head).ok()?;
    if parsed.total_len() != size {
        return None;
    }
    Some(path)
}

// ---------------------------------------------------------------------------
// Refresh (background start hook)
// ---------------------------------------------------------------------------

/// Engine start hook: refresh in the background, never block a launch.
pub async fn on_engine_start(engine: &Engine) -> EngineResult<()> {
    let e = engine.clone();
    engine.spawn_background("geoip", async move { refresh(&e).await });
    Ok(())
}

/// Download/refresh the country table when the cached one is stale.
pub async fn refresh(engine: &Engine) -> EngineResult<()> {
    refresh_from(engine, DBIP_BASE, Utc::now()).await
}

fn is_status(e: &EngineError, status: u16) -> bool {
    matches!(e, EngineError::Http(h) if h.kind == HttpErrorKind::Status(status))
}

fn age_secs(path: &Path, now: DateTime<Utc>) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let at = DateTime::<Utc>::from(modified);
    Some(now.timestamp() - at.timestamp())
}

/// [`refresh`] with an injectable base URL and clock (tests).
pub(crate) async fn refresh_from(engine: &Engine, base: &str, now: DateTime<Utc>) -> EngineResult<()> {
    let paths = engine.paths();
    let dir = paths.geoip_dir();
    paths::ensure_private_dir(&dir)?;
    let current = month_key(now);
    let previous = month_key(now.checked_sub_months(Months::new(1)).unwrap_or(now));
    let usable = || {
        // A file stamped in the future (clock skew) counts as fresh, not stale.
        table_path(paths).filter(|file| age_secs(file, now).is_some_and(|a| a <= MAX_AGE_SECS))
    };

    // Right month and a young enough file: nothing to do.
    let existing = read_meta(&dir).filter(|_| usable().is_some());
    if let Some(meta) = &existing {
        if meta.month == current {
            tracing::debug!("[geoip] reusing countries-{current}.bin");
            return Ok(());
        }
    }

    let mut month = current.clone();
    let gz = match fetch_month(engine, base, &month).await {
        Ok(b) => b,
        Err(e) if is_status(&e, 404) => {
            // Early in the month DB-IP still serves the previous release. If
            // the previous release is already here, keep it: no reason to
            // re-download 4.6 MB just to write the same table again.
            if existing.as_ref().is_some_and(|m| m.month == previous) {
                tracing::debug!("[geoip] {current} is not published yet; keeping {previous}");
                return Ok(());
            }
            if previous == month {
                return Err(e);
            }
            tracing::info!("[geoip] {month} is not published yet; using {previous}");
            month = previous.clone();
            fetch_month(engine, base, &month).await?
        }
        Err(e) if existing.is_some() => {
            // A stale-but-usable table beats no table on a flaky network.
            tracing::warn!("[geoip] keeping the cached table ({e})");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    let source = format!("{base}/dbip-country-lite-{month}.csv.gz");
    // 4.6 MB gzipped / ~40 MB of CSV: decompress + parse off the reactor.
    let build_source = source.clone();
    let (bytes, table) = tokio::task::spawn_blocking(move || build(&gz, &build_source))
        .await
        .map_err(|e| EngineError::other(format!("geoip build task failed: {e}")))??;

    let file = format!("countries-{month}.bin");
    let path = dir.join(&file);
    paths::write_private(&path, &bytes)?;
    let meta = GeoMeta {
        month,
        file,
        source,
        refreshed_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        v4_ranges: table.v4.len() as u64,
        v6_ranges: table.v6.len() as u64,
        countries: table.countries.len() as u64,
        bytes: bytes.len() as u64,
    };
    paths::write_json_atomic(&paths.geoip_meta(), &meta)?;
    prune_tables(&dir, &meta.file);
    tracing::info!(
        "[geoip] table {} ({} bytes, {} v4 + {} v6 ranges, {} countries)",
        meta.file,
        meta.bytes,
        meta.v4_ranges,
        meta.v6_ranges,
        meta.countries
    );
    Ok(())
}

async fn fetch_month(engine: &Engine, base: &str, month: &str) -> EngineResult<Vec<u8>> {
    let url = format!("{base}/dbip-country-lite-{month}.csv.gz");
    // ~4.6 MB over a cold link: the default 60 s would abort mid-download on
    // slow connections, and the monthly refresh is the only chance it gets.
    let opts = FetchOptions { timeout: std::time::Duration::from_secs(300), ..Default::default() };
    engine.http().fetch_bytes(&url, &opts).await
}

fn read_meta(dir: &Path) -> Option<GeoMeta> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).ok()?).ok()
}

fn gunzip(data: &[u8]) -> Result<String, String> {
    let mut csv = String::new();
    flate2::read::GzDecoder::new(data)
        .take(MAX_CSV_BYTES + 1)
        .read_to_string(&mut csv)
        .map_err(|e| format!("could not gunzip the CSV: {e}"))?;
    if csv.len() as u64 > MAX_CSV_BYTES {
        return Err("uncompressed CSV is implausibly large".into());
    }
    Ok(csv)
}

/// Blocking half of the refresh: gunzip → parse → merge → encode.
fn build(gz: &[u8], source: &str) -> EngineResult<(Vec<u8>, GeoTable)> {
    let fail = |e: String| EngineError::other(format!("{source}: {e}"));
    let table = parse_csv(&gunzip(gz).map_err(fail)?);
    // A real DB-IP release always has both families (2026-11: 362,984 + 369,794
    // ranges). An empty section means we were served something that is not
    // the file we asked for; keep the previous table rather than gut it.
    if table.v4.is_empty() || table.v6.is_empty() {
        return Err(fail(format!("no usable ranges ({} v4, {} v6)", table.v4.len(), table.v6.len())));
    }
    let bytes = encode(&table).map_err(fail)?;
    Ok((bytes, table))
}

/// Delete every `countries-*.bin` except `keep` (previous months).
fn prune_tables(dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        if name != keep && valid_table_name(&name) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                tracing::warn!("[geoip] could not delete {}: {e}", entry.path().display());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{temp_engine, MockResponse, MockServer};
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    const CSV: &str = "\
0.0.0.0,0.255.255.255,ZZ
1.0.0.0,1.0.0.255,AU
1.0.1.0,1.0.3.255,CN
1.0.4.0,1.0.4.255,AU
10.0.0.0,10.0.0.255,zz
8.8.8.0,8.8.8.255,US
2a00:1450::,2a00:1450:ffff:ffff:ffff:ffff:ffff:ffff,US
2a01:4f8::,2a01:4f8:ffff:ffff:ffff:ffff:ffff:ffff,DE
2001:db8::,2001:db8::ffff,ES
";

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn table() -> GeoTable {
        parse_csv(CSV)
    }

    fn gz(csv: &str) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(csv.as_bytes()).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn csv_round_trips_through_the_binary_table() {
        let t = table();
        let bytes = encode(&t).unwrap();
        assert_eq!(bytes.len(), HEADER_LEN + t.countries.len() * 2 + t.entry_count() * ENTRY_LEN);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(bytes.len() as u64, parse_header(&bytes).unwrap().total_len());
        assert_eq!(decode(&bytes).unwrap(), t);
        // Countries come out sorted and unique; ZZ never enters the table.
        assert_eq!(t.countries, vec![*b"AU", *b"CN", *b"DE", *b"ES", *b"US"]);
        assert!(t.countries.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(lookup(&t, ip("8.8.8.8")).as_deref(), Some("US"));
        assert_eq!(lookup(&t, ip("1.0.2.7")).as_deref(), Some("CN"));
        // The two sections are what the binary search assumes: ascending
        // starts and no two entries covering the same address.
        for section in [&t.v4, &t.v6] {
            assert!(section.windows(2).all(|w| w[0].start < w[1].start && w[0].end < w[1].start), "{section:?}");
        }
        // Header fields the Java reader reads back.
        assert_eq!(&bytes[8..12], &(t.v4.len() as u32).to_le_bytes());
        assert_eq!(&bytes[12..16], &(t.v6.len() as u32).to_le_bytes());
        assert_eq!(&bytes[16..20], &(t.countries.len() as u32).to_le_bytes());
        assert_eq!(&bytes[20..24], &0u32.to_le_bytes());
    }

    #[test]
    fn lookup_hits_range_boundaries_and_misses_gaps() {
        let t = table();
        // First and last address of 1.0.0.0/24 (AU).
        assert_eq!(lookup(&t, ip("1.0.0.0")).as_deref(), Some("AU"));
        assert_eq!(lookup(&t, ip("1.0.0.255")).as_deref(), Some("AU"));
        // Last address of the CN range and the first of the next AU one: the
        // ranges touch but differ in country, so no merge may happen.
        assert_eq!(lookup(&t, ip("1.0.3.255")).as_deref(), Some("CN"));
        assert_eq!(lookup(&t, ip("1.0.4.0")).as_deref(), Some("AU"));
        // Gap after 1.0.4.255 up to 8.8.8.0.
        assert_eq!(lookup(&t, ip("1.0.5.0")), None);
        assert_eq!(lookup(&t, ip("5.5.5.5")), None);
        // Before the first range / after the last one.
        assert_eq!(lookup(&t, ip("0.0.0.1")), None, "0/8 is ZZ");
        assert_eq!(lookup(&t, ip("255.255.255.254")), None);
        assert_eq!(lookup(&t, ip("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")), None);
    }

    #[test]
    fn ipv6_lookups_use_the_v6_section() {
        let t = table();
        assert_eq!(lookup(&t, ip("2a00:1450:4001::1")).as_deref(), Some("US"));
        assert_eq!(lookup(&t, ip("2a00:1450::")).as_deref(), Some("US"));
        assert_eq!(lookup(&t, ip("2a01:4f8:1:2::3")).as_deref(), Some("DE"));
        assert_eq!(lookup(&t, ip("2001:db8::ffff")).as_deref(), Some("ES"));
        assert_eq!(lookup(&t, ip("2001:db8::1:0")), None, "just past 2001:db8::ffff");
        assert_eq!(lookup(&t, ip("2a00:1451::1")), None);
        // The v4 ranges live in their own section: no cross-family hits.
        assert_eq!(lookup(&t, ip("::1.0.0.1")), None);
    }

    #[test]
    fn ipv4_is_stored_ipv4_mapped() {
        let t = table();
        let r = t.v4.iter().find(|r| r.end == mapped(Ipv4Addr::new(8, 8, 8, 255))).unwrap();
        assert_eq!(&r.start[..10], &[0u8; 10]);
        assert_eq!(&r.start[10..12], &[0xff, 0xff]);
        assert_eq!(&r.start[12..], &[8, 8, 8, 0]);
        // A mapped IPv6 address is answered from the IPv4 section.
        assert_eq!(lookup(&t, ip("::ffff:8.8.8.8")).as_deref(), Some("US"));
        assert!(t.v6.iter().all(|r| !(r.start[..10] == [0u8; 10] && r.start[10..12] == [0xff, 0xff])));
        // Everything encoded fits one shape: 16-byte big-endian endpoints.
        let bytes = encode(&t).unwrap();
        let first = &bytes[HEADER_LEN + t.countries.len() * 2..][..ENTRY_LEN];
        let index = u16::from_le_bytes([first[32], first[33]]) as usize;
        assert_eq!(&t.countries[index], b"AU", "u16 LE index into the country table");
        assert_eq!(&first[..16], &mapped(Ipv4Addr::new(1, 0, 0, 0)));
    }

    #[test]
    fn zz_ranges_are_dropped() {
        let t = table();
        // 0.0.0.0/8 and 10.0.0.0/24 ("zz") are unknown: no entry may cover them.
        for a in ["0.0.0.0", "0.1.2.3", "10.0.0.1"] {
            assert_eq!(lookup(&t, ip(a)), None, "{a} must stay unknown");
        }
        assert_eq!(t.v4.len(), 4, "AU, CN, AU, US — the ZZ rows are gone");
    }

    #[test]
    fn adjacent_same_country_ranges_merge_but_gaps_do_not() {
        let t = parse_csv("1.0.0.0,1.0.0.255,AU\n1.0.1.0,1.0.1.255,AU\n1.0.2.0,1.0.3.255,CN\n");
        assert_eq!(t.v4.len(), 2, "the two AU /24s become one range");
        assert_eq!(t.v4[0].start, mapped(Ipv4Addr::new(1, 0, 0, 0)));
        assert_eq!(t.v4[0].end, mapped(Ipv4Addr::new(1, 0, 1, 255)));
        assert_eq!(lookup(&t, ip("1.0.1.0")).as_deref(), Some("AU"));
        // Same country with a hole between must NOT merge.
        let gap = parse_csv("1.0.0.0,1.0.0.255,AU\n1.0.2.0,1.0.2.255,AU\n");
        assert_eq!(gap.v4.len(), 2);
        assert_eq!(lookup(&gap, ip("1.0.1.0")), None);
        // ff…ff has no successor: merging into it is still safe.
        let top = parse_csv("255.255.255.0,255.255.255.255,US\n255.255.255.249,255.255.255.250,US\n");
        assert_eq!(top.v4.len(), 1);
        assert_eq!(lookup(&top, ip("255.255.255.255")).as_deref(), Some("US"));
    }

    #[tokio::test]
    async fn refresh_falls_back_to_the_previous_month_on_404() {
        let (_dir, e) = temp_engine();
        let now = Utc::now();
        let prev = month_key(now.checked_sub_months(Months::new(1)).unwrap());
        let cur = month_key(now);
        let gz_body = gz(CSV);
        let cur_name = format!("{cur}.csv.gz");
        let server = MockServer::start(move |req| {
            if req.path.ends_with(&cur_name) {
                MockResponse::status(404)
            } else {
                MockResponse::bytes(gz_body.clone()).with_header("content-type", "application/gzip")
            }
        })
        .await;
        refresh_from(&e, &server.url("/free"), now).await.expect("fallback refresh");
        assert_eq!(server.hits(), 2);
        assert!(server.requests()[0].path.contains(&cur));
        assert!(server.requests()[1].path.contains(&prev));
        let path = e.paths().geoip_table(&prev);
        assert!(path.exists(), "previous month's table written");
        let meta = read_meta(&e.paths().geoip_dir()).unwrap();
        assert_eq!(meta.month, prev);
        assert_eq!(meta.file, format!("countries-{prev}.bin"));
        assert_eq!(meta.v4_ranges, 4);
        assert!(meta.bytes > 0);
        assert_eq!(table_path(e.paths()), Some(path));

        // A second refresh with the same clock keeps the table: it probes the
        // unpublished current month once (404) and does not re-download.
        refresh_from(&e, &server.url("/free"), now).await.unwrap();
        assert_eq!(server.hits(), 3, "one cheap probe, no re-download");
        assert!(server.requests()[2].path.contains(&cur));
        assert_eq!(table_path(e.paths()), Some(e.paths().geoip_table(&prev)));
    }

    #[tokio::test]
    async fn corrupt_gzip_leaves_no_table_and_no_property() {
        let (_dir, e) = temp_engine();
        let server = MockServer::start(|_| MockResponse::bytes(b"not a gzip".to_vec())).await;
        let err = refresh_from(&e, &server.url("/free"), Utc::now()).await;
        assert!(err.is_err(), "corrupt download must fail the refresh");
        assert_eq!(table_path(e.paths()), None, "no property source");
        assert!(!e.paths().geoip_meta().exists(), "meta.json only after a good table");
        let leftovers: Vec<_> = std::fs::read_dir(e.paths().geoip_dir())
            .map(|rd| rd.flatten().map(|d| d.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        assert!(leftovers.iter().all(|n| !n.starts_with("countries-")), "{leftovers:?}");
    }

    #[tokio::test]
    async fn a_bad_refresh_keeps_the_previous_table() {
        let (_dir, e) = temp_engine();
        let now = Utc::now();
        let good = MockServer::start({
            let body = gz(CSV);
            move |_| MockResponse::bytes(body.clone())
        })
        .await;
        refresh_from(&e, &good.url("/free"), now).await.unwrap();
        let before = table_path(e.paths());
        assert!(before.is_some());
        // Next month: the server answers garbage. The old table must survive.
        let bad = MockServer::start(|_| MockResponse::bytes(b"junk".to_vec())).await;
        let later = now + chrono::Duration::days(40);
        assert!(refresh_from(&e, &bad.url("/free"), later).await.is_err());
        assert_eq!(table_path(e.paths()), before);
    }

    #[test]
    fn table_path_rejects_a_broken_pointer() {
        let (_dir, e) = temp_engine();
        let paths = e.paths();
        let good = encode(&table()).unwrap();
        paths::write_private(&paths.geoip_table("2026-11"), &good).unwrap();
        let ok = serde_json::json!({ "month": "2026-11", "file": "countries-2026-11.bin" });
        // No meta at all, a path escape, and a name that is not a table file.
        assert_eq!(table_path(paths), None, "no meta.json");
        for bad in [
            serde_json::json!({ "month": "2026-11", "file": "../escape.bin" }),
            serde_json::json!({ "month": "2026-11", "file": "countries-2026-11.bin.x" }),
        ] {
            paths::write_json_atomic(&paths.geoip_meta(), &bad).unwrap();
            assert_eq!(table_path(paths), None, "{bad}");
        }
        // Truncated file: the header length disagrees with the size.
        paths::write_private(&paths.geoip_table("2026-11"), &good[..good.len() - 1]).unwrap();
        paths::write_json_atomic(&paths.geoip_meta(), &ok).unwrap();
        assert_eq!(table_path(paths), None);
        paths::write_private(&paths.geoip_table("2026-11"), &good).unwrap();
        assert_eq!(table_path(paths), Some(paths.geoip_table("2026-11")));
    }

    #[test]
    fn prune_keeps_only_the_current_table() {
        let (_dir, e) = temp_engine();
        let paths = e.paths();
        let body = encode(&table()).unwrap();
        for m in ["2026-09", "2026-10", "2026-11"] {
            paths::write_private(&paths.geoip_table(m), &body).unwrap();
        }
        prune_tables(&paths.geoip_dir(), "countries-2026-11.bin");
        let mut left: Vec<String> = std::fs::read_dir(paths.geoip_dir())
            .unwrap()
            .flatten()
            .map(|d| d.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["countries-2026-11.bin".to_string()]);
    }

    #[test]
    fn months_and_names_validate() {
        let at = DateTime::parse_from_rfc3339("2026-01-15T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(month_key(at), "2026-01");
        assert_eq!(month_key(at.checked_sub_months(Months::new(1)).unwrap()), "2025-12");
        assert!(valid_table_name("countries-2026-11.bin"));
        assert!(!valid_table_name("countries-2026-13.bin"));
        assert!(!valid_table_name("countries-2026-11.bin.tmp"));
        assert!(!valid_table_name("countries-X.bin"));
        assert!(SystemTime::now().duration_since(UNIX_EPOCH).is_ok());
    }
}
