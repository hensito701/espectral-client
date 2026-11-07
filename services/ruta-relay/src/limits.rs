//! In-memory limit accounting: global + per source-IP concurrency at accept,
//! per-`sub` concurrency/rate/quota once the grant is verified.
//!
//! Everything lives in one `Mutex`-guarded map. At the documented caps (400
//! global, 8 per IP, 4 per sub) the critical section is a few hash lookups on a
//! tiny map, so sharded locks or atomics would only add complexity.
//!
//! [`Slot`] is RAII: dropping it returns every reservation, including when the
//! connection task is cancelled mid-flight.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use crate::util::now_unix;

/// Sliding window for the per-sub connection rate.
const RATE_WINDOW_S: i64 = 60;

/// Unix seconds at UTC midnight of the day containing `now`.
fn utc_day_start(now: i64) -> i64 {
    now.div_euclid(86_400) * 86_400
}

#[derive(Debug, Default)]
struct SubEntry {
    concurrent: usize,
    /// Start of the current UTC day and bytes consumed in it.
    day: i64,
    day_bytes: u64,
    /// Start timestamps of accepted connections inside the rate window.
    starts: Vec<i64>,
}

#[derive(Debug, Default)]
struct Inner {
    global: usize,
    subs: HashMap<String, SubEntry>,
    ips: HashMap<IpAddr, usize>,
}

/// Why an acquire failed; maps onto `ERR busy` / `ERR limit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// Global concurrency cap reached (`busy`).
    Busy,
    /// Per-IP concurrency, per-sub concurrency, per-sub rate or quota hit
    /// (`limit`).
    Limit,
}

/// Shared limit state. Cheap to clone (`Arc`).
#[derive(Clone)]
pub struct Limits {
    inner: Arc<Mutex<Inner>>,
    pub per_sub_concurrency: usize,
    pub per_sub_rate_per_min: u32,
    pub per_sub_daily_bytes: u64,
    pub per_ip_concurrency: usize,
    pub global_max: usize,
}

/// A held reservation for one connection.
pub struct Slot {
    limits: Limits,
    ip: IpAddr,
    sub: Option<String>,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("ip", &self.ip)
            .field("sub", &self.sub)
            .finish()
    }
}

impl Slot {
    /// Charges `bytes` against the sub's daily quota *before* they are
    /// forwarded. Returns `false` when the budget is exhausted, in which case
    /// the caller must end the session with `limit`.
    pub fn charge(&self, bytes: u64) -> bool {
        let Some(sub) = self.sub.as_deref() else {
            return true; // pre-auth bytes are not billed
        };
        let mut g = self.limits.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = g.subs.get_mut(sub) else {
            return false;
        };
        let today = utc_day_start(now_unix());
        if entry.day != today {
            entry.day = today;
            entry.day_bytes = 0;
        }
        // Committed only when the whole chunk fits: a refused chunk must not
        // silently inflate the budget it just failed against.
        let next = entry.day_bytes.saturating_add(bytes);
        if next > self.limits.per_sub_daily_bytes {
            return false;
        }
        entry.day_bytes = next;
        true
    }

    /// Bytes this sub already used today.
    pub fn used_today(&self) -> u64 {
        let Some(sub) = self.sub.as_deref() else {
            return 0;
        };
        let mut g = self.limits.inner.lock().unwrap_or_else(|e| e.into_inner());
        let today = utc_day_start(now_unix());
        match g.subs.get_mut(sub) {
            Some(e) => {
                if e.day != today {
                    e.day = today;
                    e.day_bytes = 0;
                }
                e.day_bytes
            }
            None => 0,
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut g = self.limits.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.global = g.global.saturating_sub(1);
        if let Some(n) = g.ips.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                g.ips.remove(&self.ip);
            }
        }
        if let Some(sub) = &self.sub {
            if let Some(e) = g.subs.get_mut(sub) {
                e.concurrent = e.concurrent.saturating_sub(1);
                if e.concurrent == 0 && e.day_bytes == 0 && e.starts.is_empty() {
                    g.subs.remove(sub);
                }
            }
        }
    }
}

/// Point-in-time counters (used by the log line and tests).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub global: usize,
    pub subs: usize,
    pub ips: usize,
}

impl Limits {
    pub fn new(
        per_sub_concurrency: usize,
        per_sub_rate_per_min: u32,
        per_sub_daily_bytes: u64,
        per_ip_concurrency: usize,
        global_max: usize,
    ) -> Self {
        Limits {
            inner: Arc::new(Mutex::new(Inner::default())),
            per_sub_concurrency,
            per_sub_rate_per_min,
            per_sub_daily_bytes,
            per_ip_concurrency,
            global_max,
        }
    }

    /// Reserves the global and per-source-IP slots, before any crypto runs.
    pub fn accept(&self, ip: IpAddr) -> Result<Slot, Reject> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.global >= self.global_max {
            return Err(Reject::Busy);
        }
        // `get` first: a rejected connection must not leave a stray zero entry
        // behind (source addresses are attacker-chosen and unbounded).
        if g.ips.get(&ip).copied().unwrap_or(0) >= self.per_ip_concurrency {
            return Err(Reject::Limit);
        }
        *g.ips.entry(ip).or_insert(0) += 1;
        g.global += 1;
        Ok(Slot {
            limits: self.clone(),
            ip,
            sub: None,
        })
    }

    /// Binds a verified `sub` to the reservation: per-sub concurrency, the
    /// one-minute rate window and the daily quota. A rejected attempt leaves
    /// no trace (no rate slot consumed, no empty map entry).
    pub fn attach_sub(&self, slot: &mut Slot, sub: &str) -> Result<(), Reject> {
        debug_assert!(slot.sub.is_none(), "sub attached twice");
        let now = now_unix();
        let today = utc_day_start(now);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        match g.subs.get_mut(sub) {
            Some(entry) => {
                if entry.concurrent >= self.per_sub_concurrency {
                    return Err(Reject::Limit);
                }
                entry.starts.retain(|t| *t > now - RATE_WINDOW_S);
                if entry.starts.len() >= self.per_sub_rate_per_min as usize {
                    return Err(Reject::Limit);
                }
                if entry.day != today {
                    entry.day = today;
                    entry.day_bytes = 0;
                }
                if entry.day_bytes >= self.per_sub_daily_bytes {
                    return Err(Reject::Limit);
                }
                entry.concurrent += 1;
                entry.starts.push(now);
            }
            None => {
                // First sighting of this sub today: nothing to exceed yet, but
                // an exhausted *previous* day still starts clean at midnight.
                let mut entry = SubEntry {
                    concurrent: 1,
                    day: today,
                    ..SubEntry::default()
                };
                entry.starts.push(now);
                g.subs.insert(sub.to_owned(), entry);
            }
        }
        slot.sub = Some(sub.to_owned());
        Ok(())
    }

    /// Drops expired rate-window entries and resets stale day counters so the
    /// maps stay proportional to recent traffic, not to uptime. Called once a
    /// minute by the server's maintenance task.
    pub fn prune(&self) {
        let now = now_unix();
        let today = utc_day_start(now);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.subs.retain(|_, e| {
            if e.day != today {
                e.day = today;
                e.day_bytes = 0;
            }
            e.starts.retain(|t| *t > now - RATE_WINDOW_S);
            e.concurrent > 0 || e.day_bytes > 0 || !e.starts.is_empty()
        });
    }

    /// Read-only counters (tests and diagnostics).
    pub fn snapshot(&self) -> Snapshot {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            global: g.global,
            subs: g.subs.len(),
            ips: g.ips.len(),
        }
    }

    /// Test seam: pretend `sub` already burned `bytes` today.
    #[doc(hidden)]
    pub fn preload_day_bytes(&self, sub: &str, bytes: u64) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let entry = g.subs.entry(sub.to_owned()).or_default();
        entry.day = utc_day_start(now_unix());
        entry.day_bytes = bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([198, 51, 100, n])
    }

    fn with_sub(l: &Limits, sub: &str, addr: u8) -> Slot {
        let mut s = l.accept(ip(addr)).unwrap();
        l.attach_sub(&mut s, sub).unwrap();
        s
    }

    #[test]
    fn sub_concurrency_limit() {
        let l = Limits::new(2, 100, u64::MAX, 100, 100);
        let mut a = l.accept(ip(1)).unwrap();
        l.attach_sub(&mut a, "s").unwrap();
        let _b = with_sub(&l, "s", 1);
        let mut c = l.accept(ip(1)).unwrap();
        assert_eq!(l.attach_sub(&mut c, "s").unwrap_err(), Reject::Limit);
        drop(a);
        assert!(l.attach_sub(&mut c, "s").is_ok());
    }

    #[test]
    fn rate_limit_within_minute() {
        let l = Limits::new(100, 3, u64::MAX, 100, 100);
        let now = now_unix();
        {
            let mut g = l.inner.lock().unwrap();
            let e = g.subs.entry("s".into()).or_default();
            e.starts = vec![now - 5, now - 10, now - 20];
        }
        let mut c = l.accept(ip(1)).unwrap();
        assert_eq!(l.attach_sub(&mut c, "s").unwrap_err(), Reject::Limit);
        // Entries outside the window no longer count.
        {
            let mut g = l.inner.lock().unwrap();
            let e = g.subs.get_mut("s").unwrap();
            e.starts = vec![now - 61, now - 62, now - 63];
        }
        assert!(l.attach_sub(&mut c, "s").is_ok());
    }

    #[test]
    fn ip_concurrency_limit_is_per_ip() {
        let l = Limits::new(100, 100, u64::MAX, 1, 100);
        let _a = with_sub(&l, "s1", 1);
        assert_eq!(l.accept(ip(1)).unwrap_err(), Reject::Limit);
        assert!(l.accept(ip(2)).is_ok());
    }

    #[test]
    fn rejected_accepts_leave_no_ip_entry() {
        let l = Limits::new(100, 100, u64::MAX, 1, 100);
        let a = l.accept(ip(1)).unwrap();
        // Per-IP rejection from a *new* source must not create a zero entry.
        assert_eq!(l.accept(ip(1)).unwrap_err(), Reject::Limit);
        assert_eq!(l.snapshot().ips, 1, "only the live IP is mapped");
        drop(a);
        assert_eq!(l.snapshot().ips, 0);

        // Same for a global-cap rejection with no live slots anywhere.
        let g = Limits::new(100, 100, u64::MAX, 100, 1);
        let b = g.accept(ip(7)).unwrap();
        assert_eq!(g.accept(ip(8)).unwrap_err(), Reject::Busy);
        assert_eq!(g.snapshot().ips, 1);
        drop(b);
        assert_eq!(g.snapshot().ips, 0);
    }

    #[test]
    fn global_cap() {
        let l = Limits::new(100, 100, u64::MAX, 100, 2);
        let a = with_sub(&l, "a", 1);
        let b = with_sub(&l, "b", 2);
        assert_eq!(l.accept(ip(3)).unwrap_err(), Reject::Busy);
        drop(a);
        assert!(l.accept(ip(3)).is_ok());
        drop(b);
    }

    #[test]
    fn daily_quota() {
        let l = Limits::new(4, 100, 1000, 100, 100);
        let s = with_sub(&l, "s", 1);
        assert!(s.charge(600));
        assert!(s.charge(400));
        assert_eq!(s.used_today(), 1000);
        assert!(!s.charge(1), "over quota");
        assert_eq!(s.used_today(), 1000, "a refused chunk must not be billed");
        assert!(s.charge(0), "a zero-byte chunk always fits");
        // An exhausted sub cannot open a new connection either.
        let mut c = l.accept(ip(2)).unwrap();
        assert_eq!(l.attach_sub(&mut c, "s").unwrap_err(), Reject::Limit);
    }

    #[test]
    fn quota_resets_on_utc_midnight() {
        let l = Limits::new(4, 100, 1000, 100, 100);
        {
            let mut g = l.inner.lock().unwrap();
            let e = g.subs.entry("s".into()).or_default();
            e.day = utc_day_start(now_unix()) - 86_400;
            e.day_bytes = 999_999;
        }
        let s = with_sub(&l, "s", 1);
        assert_eq!(s.used_today(), 0);
        assert!(s.charge(1000));
    }

    #[test]
    fn day_start_is_midnight() {
        assert_eq!(utc_day_start(0), 0);
        assert_eq!(utc_day_start(86_399), 0);
        assert_eq!(utc_day_start(86_400), 86_400);
        assert_eq!(utc_day_start(-1), -86_400);
        // 2026-11-07T12:00:00Z
        assert_eq!(utc_day_start(1_794_052_800), 1_794_009_600);
    }

    #[test]
    fn slots_release_on_drop() {
        let l = Limits::new(4, 100, u64::MAX, 4, 4);
        {
            let s = with_sub(&l, "s", 1);
            s.charge(10);
            let snap = l.snapshot();
            assert_eq!(snap.global, 1);
            assert_eq!(snap.ips, 1);
            assert_eq!(snap.subs, 1);
        }
        let snap = l.snapshot();
        assert_eq!(snap.global, 0);
        assert_eq!(snap.ips, 0);
        // The sub entry survives only while it has quota charged today.
        assert_eq!(snap.subs, 1);
    }

    #[test]
    fn dropped_slot_keeps_rate_window_then_prunes() {
        let l = Limits::new(4, 100, u64::MAX, 4, 4);
        drop(with_sub(&l, "s", 1));
        // The start timestamp stays for the one-minute window, so a burst
        // cannot bypass the rate limit by closing and reopening.
        assert_eq!(l.snapshot().subs, 1);
        assert_eq!(l.snapshot().global, 0);

        // Once the window has passed, maintenance drops the entry.
        {
            let mut g = l.inner.lock().unwrap();
            let e = g.subs.get_mut("s").unwrap();
            e.starts = vec![now_unix() - 120];
        }
        l.prune();
        assert_eq!(l.snapshot().subs, 0);
    }

    #[test]
    fn prune_resets_stale_day_and_caps_maps() {
        let l = Limits::new(4, 100, 1000, 4, 4);
        {
            let mut g = l.inner.lock().unwrap();
            let e = g.subs.entry("old".into()).or_default();
            e.day = utc_day_start(now_unix()) - 86_400;
            e.day_bytes = 10_000;
            e.starts = vec![now_unix() - 300];
        }
        l.prune();
        // A stale day means yesterday's usage no longer blocks today.
        let mut c = l.accept(ip(1)).unwrap();
        assert!(l.attach_sub(&mut c, "old").is_ok());
        assert!(c.charge(1000));
    }

    #[test]
    fn preload_seam() {
        let l = Limits::new(4, 100, 500, 4, 4);
        l.preload_day_bytes("s", 500);
        let mut c = l.accept(ip(1)).unwrap();
        assert_eq!(l.attach_sub(&mut c, "s").unwrap_err(), Reject::Limit);
    }
}
