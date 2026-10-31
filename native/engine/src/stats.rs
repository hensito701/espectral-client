//! Port of `src/engine/stats.mjs` — launch-timing statistics.
//!
//! One JSON object per line in `<data>/launch-stats.jsonl`, newest last:
//! `{ key, instance, version, started_at, menu_at, menu_ms, played_ms, spawn_ms, boot_ms, phases }`.
//!
//! Writes are best-effort (failures are logged, never returned into the
//! launch flow); reads never fail on a missing or corrupt file. The file is
//! bounded: once it grows past 256 KiB it is rewritten to its last 500 records.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::paths::Paths;
use crate::types::LaunchStat;

/// L7: keep at most this many records after a rotation.
pub const MAX_STATS_RECORDS: usize = 500;
/// Rotation threshold and read window.
pub const STATS_WINDOW_BYTES: u64 = 256 * 1024;

/// Append one launch record (one JSON line). Never fails.
pub fn record_launch_stat(paths: &Paths, stat: &LaunchStat) {
    if let Err(e) = record_inner(&paths.launch_stats(), stat) {
        tracing::warn!("[stats] could not record launch stat: {e}");
    }
}

fn record_inner(file: &Path, stat: &LaunchStat) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(stat).map_err(std::io::Error::other)?;
    {
        let mut f = OpenOptions::new().create(true).append(true).open(file)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
    }
    let size = fs::metadata(file)?.len();
    if size > STATS_WINDOW_BYTES {
        let text = fs::read_to_string(file)?;
        let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
        if lines.len() > MAX_STATS_RECORDS {
            let kept = &lines[lines.len() - MAX_STATS_RECORDS..];
            fs::write(file, kept.join("\n") + "\n")?;
        }
    }
    Ok(())
}

/// The newest `limit` records, newest first. Never fails (missing/corrupt -> []).
pub fn read_launch_stats(paths: &Paths, limit: usize) -> Vec<LaunchStat> {
    read_inner(&paths.launch_stats(), limit).unwrap_or_default()
}

fn read_inner(file: &Path, limit: usize) -> std::io::Result<Vec<LaunchStat>> {
    if !file.exists() {
        return Ok(vec![]);
    }
    let mut f = fs::File::open(file)?;
    let size = f.metadata()?.len();
    let read_size = size.min(STATS_WINDOW_BYTES);
    let offset = size - read_size;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::with_capacity(read_size as usize);
    f.take(read_size).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    // Only a tail read can begin mid-line: drop that first fragment then.
    let slice: &str = if offset > 0 {
        match text.find('\n') {
            Some(i) => &text[i + 1..],
            None => &text,
        }
    } else {
        &text
    };
    let mut records: Vec<LaunchStat> = slice
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.is_object())
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    let start = records.len().saturating_sub(limit);
    records.drain(..start);
    records.reverse();
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(
        key: &str,
        instance: &str,
        version: &str,
        started_at: i64,
        menu_at: Option<i64>,
        menu_ms: Option<i64>,
        played_ms: i64,
    ) -> LaunchStat {
        LaunchStat {
            key: key.into(),
            instance: instance.into(),
            version: Some(version.into()),
            started_at: Some(started_at),
            menu_at,
            menu_ms,
            played_ms: Some(played_ms),
            ..Default::default()
        }
    }

    #[test]
    fn missing_file_returns_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(read_launch_stats(&Paths::new(d.path()), 10).is_empty());
    }

    #[test]
    fn round_trip_newest_first_with_limit_corrupt_lines_and_shapes() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        let a = rec("a", "one", "1.21.11", 1000, None, None, 12000);
        let b = rec("b", "two", "26.2", 2000, Some(6000), Some(4000), 20000);
        let c = rec("c", "three", "1.20.4", 3000, None, None, 8000);
        record_launch_stat(&p, &a);
        record_launch_stat(&p, &b);
        record_launch_stat(&p, &c);
        let all = read_launch_stats(&p, 10);
        assert_eq!(all.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
        assert_eq!(all[1], b);
        assert_eq!(read_launch_stats(&p, 2).iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["c", "b"]);

        // corrupt JSON lines are skipped
        let mut f = OpenOptions::new().append(true).open(p.launch_stats()).unwrap();
        f.write_all(b"this is not json\n{\"key\": \"partial\", \"broken\": \n").unwrap();
        drop(f);
        record_launch_stat(&p, &rec("d", "four", "21.0", 4000, None, None, 5000));
        assert_eq!(read_launch_stats(&p, 10).iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["d", "c", "b", "a"]);

        // default limit of 10
        for i in 0..12 {
            record_launch_stat(&p, &rec(&format!("bulk-{i}"), "bulk", "1.21.11", i, None, None, 1));
        }
        let r = read_launch_stats(&p, 10);
        assert_eq!(r.len(), 10);
        assert_eq!(r[0].key, "bulk-11");

        // new shape (spawn_ms/boot_ms/phases) is preserved
        let mut n = rec("new-shape-1", "inst", "1.21.11", 5000, Some(9000), Some(4000), 15000);
        n.spawn_ms = Some(120);
        n.boot_ms = Some(3880);
        n.phases = Some(json!({ "loading_mc": 5100, "mixin": 5200, "openal": 8900 }).as_object().unwrap().clone());
        record_launch_stat(&p, &n);

        // legacy rows without the new fields still read
        let mut f = OpenOptions::new().append(true).open(p.launch_stats()).unwrap();
        writeln!(f, "{}", json!({ "key": "legacy-1", "instance": "legacy", "version": "1.20.1", "started_at": 999, "menu_at": null, "menu_ms": null, "played_ms": 1234 })).unwrap();
        drop(f);
        let all = read_launch_stats(&p, 30);
        let found = all.iter().find(|r| r.key == "new-shape-1").unwrap();
        assert_eq!(found.spawn_ms, Some(120));
        assert_eq!(found.boot_ms, Some(3880));
        assert_eq!(found.phases.as_ref().unwrap()["openal"], 8900);
        let legacy = all.iter().find(|r| r.key == "legacy-1").unwrap();
        assert_eq!((legacy.spawn_ms, legacy.boot_ms, legacy.phases.is_none()), (None, None, true));
        assert!(all.iter().any(|r| r.key == "a"));
    }

    #[test]
    fn file_is_rotated_past_the_window() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        let mut r = rec("x", "i", "v", 0, None, None, 0);
        r.extra.insert("pad".into(), json!("p".repeat(600)));
        for i in 0..520 {
            r.key = format!("k{i}");
            record_launch_stat(&p, &r);
        }
        let text = fs::read_to_string(p.launch_stats()).unwrap();
        let n = text.lines().count();
        assert!((400..=MAX_STATS_RECORDS + 1).contains(&n), "{n}");
        assert_eq!(read_launch_stats(&p, 1)[0].key, "k519");
    }
}
