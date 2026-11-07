//! Command line + environment configuration.
//!
//! Every flag has an `RUTA_*` environment fallback so the systemd unit can be
//! a single `ExecStart=` line. Precedence: command line, then environment,
//! then the documented default. No config file is read or written.

use std::net::SocketAddr;
use std::time::Duration;

/// Default listen address, per `PROTOCOL.md`.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:25565";
pub const DEFAULT_PER_SUB_CONCURRENCY: usize = 4;
pub const DEFAULT_PER_SUB_RATE_PER_MIN: u32 = 30;
pub const DEFAULT_PER_SUB_DAILY_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB
pub const DEFAULT_PER_IP_CONCURRENCY: usize = 8;
pub const DEFAULT_GLOBAL_MAX: usize = 400;
pub const DEFAULT_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub const DEFAULT_WORKERS: usize = 1;

/// Fully resolved runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: Vec<SocketAddr>,
    pub per_sub_concurrency: usize,
    pub per_sub_rate_per_min: u32,
    pub per_sub_daily_bytes: u64,
    pub per_ip_concurrency: usize,
    pub global_max: usize,
    pub header_timeout: Duration,
    pub connect_timeout: Duration,
    pub idle_timeout: Duration,
    pub workers: usize,
    /// Test-only: allows 127/8 and ::1 destinations. Never set in production.
    pub allow_loopback_destinations: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: vec![DEFAULT_LISTEN.parse().expect("valid default")],
            per_sub_concurrency: DEFAULT_PER_SUB_CONCURRENCY,
            per_sub_rate_per_min: DEFAULT_PER_SUB_RATE_PER_MIN,
            per_sub_daily_bytes: DEFAULT_PER_SUB_DAILY_BYTES,
            per_ip_concurrency: DEFAULT_PER_IP_CONCURRENCY,
            global_max: DEFAULT_GLOBAL_MAX,
            header_timeout: DEFAULT_HEADER_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            workers: DEFAULT_WORKERS,
            allow_loopback_destinations: false,
        }
    }
}

/// What the process should do after parsing arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(Config),
    Help,
    Version,
}

pub const USAGE: &str = "\
ruta-relay — Ruta Espectral Minecraft relay (ERT1)

Usage: ruta-relay [OPTIONS]

Options (env fallback in brackets, command line wins):
  --listen <IP:PORT>            Repeatable. [RUTA_LISTEN, comma-separated]
                                Default 0.0.0.0:25565
  --per-sub-concurrency <N>     Concurrent connections per grant sub. [RUTA_PER_SUB_CONCURRENCY] (4)
  --per-sub-rate <N>            New connections per minute per sub. [RUTA_PER_SUB_RATE_PER_MIN] (30)
  --per-sub-daily-bytes <SIZE>  Daily byte budget per sub, UTC day. [RUTA_PER_SUB_DAILY_BYTES] (8GiB)
  --per-ip-concurrency <N>      Concurrent connections per source IP. [RUTA_PER_IP_CONCURRENCY] (8)
  --global-max <N>              Global concurrent connection cap. [RUTA_GLOBAL_MAX] (400)
  --header-timeout <DURATION>   Handshake line deadline. [RUTA_HEADER_TIMEOUT] (10s)
  --connect-timeout <DURATION>  Upstream TCP connect deadline. [RUTA_CONNECT_TIMEOUT] (8s)
  --idle-timeout <DURATION>     Close after this long with no bytes either way. [RUTA_IDLE_TIMEOUT] (300s)
  --workers <N>                 Tokio worker threads. [RUTA_WORKERS] (1)
  --allow-loopback-destinations Test only: permit 127/8 and ::1 destinations.
                                [RUTA_ALLOW_LOOPBACK_DESTINATIONS] (off)
  -h, --help                    Print this help.
  -V, --version                 Print the version.

Sizes accept K/M/G/T suffixes (binary) or plain bytes; durations accept
ms/s/m/h or plain seconds. Nothing is ever written to disk; one line per
connection end goes to stdout.
";

/// Parses the process arguments (without `argv[0]`) plus the environment.
pub fn parse<I, F>(args: I, env: F) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
    F: Fn(&str) -> Option<String>,
{
    let mut listen: Vec<String> = Vec::new();
    let mut cfg = Config::default();
    let mut listen_set = false;
    // Fields given explicitly on the command line; the environment may only
    // fill in the rest.
    let mut set: std::collections::HashSet<&'static str> = std::collections::HashSet::new();

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = |flag: &str| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => it
                    .next()
                    .ok_or_else(|| format!("{flag} needs a value (see --help)")),
            }
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--listen" => {
                let v = value("--listen")?;
                listen_set = true;
                for part in v.split([',', ' ']).filter(|s| !s.is_empty()) {
                    listen.push(part.to_owned());
                }
            }
            "--per-sub-concurrency" => {
                cfg.per_sub_concurrency = parse_num(&value("--per-sub-concurrency")?)?;
                set.insert("per_sub_concurrency");
            }
            "--per-sub-rate" => {
                cfg.per_sub_rate_per_min = parse_num(&value("--per-sub-rate")?)?;
                set.insert("per_sub_rate_per_min");
            }
            "--per-sub-daily-bytes" => {
                cfg.per_sub_daily_bytes = parse_bytes(&value("--per-sub-daily-bytes")?)?;
                set.insert("per_sub_daily_bytes");
            }
            "--per-ip-concurrency" => {
                cfg.per_ip_concurrency = parse_num(&value("--per-ip-concurrency")?)?;
                set.insert("per_ip_concurrency");
            }
            "--global-max" => {
                cfg.global_max = parse_num(&value("--global-max")?)?;
                set.insert("global_max");
            }
            "--header-timeout" => {
                cfg.header_timeout = parse_duration(&value("--header-timeout")?)?;
                set.insert("header_timeout");
            }
            "--connect-timeout" => {
                cfg.connect_timeout = parse_duration(&value("--connect-timeout")?)?;
                set.insert("connect_timeout");
            }
            "--idle-timeout" => {
                cfg.idle_timeout = parse_duration(&value("--idle-timeout")?)?;
                set.insert("idle_timeout");
            }
            "--workers" => {
                cfg.workers = parse_num(&value("--workers")?)?;
                set.insert("workers");
            }
            "--allow-loopback-destinations" => {
                match inline.as_deref() {
                    None | Some("1") | Some("true") | Some("yes") => {
                        cfg.allow_loopback_destinations = true
                    }
                    Some("0") | Some("false") | Some("no") => {
                        cfg.allow_loopback_destinations = false
                    }
                    Some(other) => {
                        return Err(format!(
                            "--allow-loopback-destinations: expected 0/1, got {other:?}"
                        ))
                    }
                }
                set.insert("allow_loopback_destinations");
            }
            other => return Err(format!("unknown argument {other:?} (see --help)")),
        }
    }

    if !listen_set {
        if let Some(env_listen) = env("RUTA_LISTEN").filter(|s| !s.trim().is_empty()) {
            for part in env_listen.split([',', ' ']).filter(|s| !s.is_empty()) {
                listen.push(part.to_owned());
            }
        }
    }

    macro_rules! env_or {
        ($var:literal, $field:ident, $parse:ident) => {
            if !set.contains(stringify!($field)) {
                if let Some(raw) = env($var).filter(|s| !s.trim().is_empty()) {
                    cfg.$field = $parse(raw.trim()).map_err(|e| format!("{}: {e}", $var))?;
                }
            }
        };
    }
    // An explicit command-line value always wins: `set` records those.
    env_or!("RUTA_PER_SUB_CONCURRENCY", per_sub_concurrency, parse_num);
    env_or!("RUTA_PER_SUB_RATE_PER_MIN", per_sub_rate_per_min, parse_num);
    env_or!("RUTA_PER_SUB_DAILY_BYTES", per_sub_daily_bytes, parse_bytes);
    env_or!("RUTA_PER_IP_CONCURRENCY", per_ip_concurrency, parse_num);
    env_or!("RUTA_GLOBAL_MAX", global_max, parse_num);
    env_or!("RUTA_HEADER_TIMEOUT", header_timeout, parse_duration);
    env_or!("RUTA_CONNECT_TIMEOUT", connect_timeout, parse_duration);
    env_or!("RUTA_IDLE_TIMEOUT", idle_timeout, parse_duration);
    env_or!("RUTA_WORKERS", workers, parse_num);
    if !set.contains("allow_loopback_destinations") {
        if let Some(raw) = env("RUTA_ALLOW_LOOPBACK_DESTINATIONS").filter(|s| !s.trim().is_empty())
        {
            cfg.allow_loopback_destinations = parse_bool(raw.trim())
                .map_err(|e| format!("RUTA_ALLOW_LOOPBACK_DESTINATIONS: {e}"))?;
        }
    }

    let names: Vec<String> = if listen.is_empty() {
        vec![DEFAULT_LISTEN.to_owned()]
    } else {
        listen
    };
    let mut addrs = Vec::new();
    for name in names {
        let parsed: SocketAddr = name
            .parse()
            .map_err(|_| format!("--listen {name:?}: expected IP:PORT (e.g. 0.0.0.0:25565)"))?;
        if !addrs.contains(&parsed) {
            addrs.push(parsed);
        }
    }
    // A hostname would need DNS; anything that is not IP:PORT was rejected
    // above, so the listener never resolves a name.
    cfg.listen = addrs;

    validate(&cfg)?;
    Ok(Command::Run(cfg))
}

fn validate(cfg: &Config) -> Result<(), String> {
    if cfg.listen.is_empty() {
        return Err("at least one --listen address is required".into());
    }
    if cfg.per_sub_concurrency == 0 {
        return Err("--per-sub-concurrency must be at least 1".into());
    }
    if cfg.per_sub_rate_per_min == 0 {
        return Err("--per-sub-rate must be at least 1".into());
    }
    if cfg.per_sub_daily_bytes == 0 {
        return Err("--per-sub-daily-bytes must be greater than 0".into());
    }
    if cfg.per_ip_concurrency == 0 {
        return Err("--per-ip-concurrency must be at least 1".into());
    }
    if cfg.global_max == 0 {
        return Err("--global-max must be at least 1".into());
    }
    if cfg.workers == 0 {
        return Err("--workers must be at least 1".into());
    }
    if cfg.idle_timeout.is_zero() || cfg.connect_timeout.is_zero() || cfg.header_timeout.is_zero() {
        return Err("timeouts must be greater than zero".into());
    }
    Ok(())
}

fn parse_num<T: std::str::FromStr>(raw: &str) -> Result<T, String> {
    raw.trim()
        .parse::<T>()
        .map_err(|_| format!("invalid number {raw:?}"))
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("invalid boolean {other:?}")),
    }
}

/// `8GiB`, `8g`, `8589934592` …
pub fn parse_bytes(raw: &str) -> Result<u64, String> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(n) = s.strip_suffix("ib") {
        (n, 1)
    } else if let Some(n) = s.strip_suffix('b') {
        (n, 1)
    } else {
        (s.as_str(), 1)
    };
    let (num, mult) = match num.chars().last() {
        Some('k') => (&num[..num.len() - 1], 1024u64),
        Some('m') => (&num[..num.len() - 1], 1024 * 1024),
        Some('g') => (&num[..num.len() - 1], 1024 * 1024 * 1024),
        Some('t') => (&num[..num.len() - 1], 1024u64.pow(4)),
        _ => (num, mult),
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid size {raw:?} (bytes, or K/M/G/T suffix)"))?;
    n.checked_mul(mult)
        .ok_or_else(|| format!("size {raw:?} is too large"))
}

/// `300`, `300s`, `5m`, `500ms`, `1h`.
pub fn parse_duration(raw: &str) -> Result<Duration, String> {
    let s = raw.trim().to_ascii_lowercase();
    let (num, unit_ms) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000)
    } else {
        (s.as_str(), 1000)
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid duration {raw:?} (ms/s/m/h, or seconds)"))?;
    let ms = n
        .checked_mul(unit_ms)
        .ok_or_else(|| format!("duration {raw:?} is too large"))?;
    Ok(Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str], env: &[(&str, &str)]) -> Result<Command, String> {
        let env_map: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        parse(args.iter().map(|s| s.to_string()), |k| {
            env_map
                .iter()
                .find(|(ek, _)| ek == k)
                .map(|(_, v)| v.clone())
        })
    }

    fn cfg(args: &[&str], env: &[(&str, &str)]) -> Config {
        match parse_args(args, env).unwrap() {
            Command::Run(c) => c,
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn defaults_match_protocol() {
        let c = cfg(&[], &[]);
        assert_eq!(c.listen, vec!["0.0.0.0:25565".parse().unwrap()]);
        assert_eq!(c.per_sub_concurrency, 4);
        assert_eq!(c.per_sub_rate_per_min, 30);
        assert_eq!(c.per_sub_daily_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(c.per_ip_concurrency, 8);
        assert_eq!(c.global_max, 400);
        assert_eq!(c.header_timeout, Duration::from_secs(10));
        assert_eq!(c.connect_timeout, Duration::from_secs(8));
        assert_eq!(c.idle_timeout, Duration::from_secs(300));
        assert!(!c.allow_loopback_destinations);
    }

    #[test]
    fn repeatable_listen_and_equals_form() {
        let c = cfg(&["--listen", "0.0.0.0:25565", "--listen=0.0.0.0:443"], &[]);
        assert_eq!(c.listen.len(), 2);
        assert_eq!(c.listen[1].port(), 443);

        let c = cfg(&["--listen", "127.0.0.1:1,127.0.0.1:2"], &[]);
        assert_eq!(c.listen.len(), 2);

        // Duplicates collapse.
        let c = cfg(
            &["--listen", "0.0.0.0:25565", "--listen=0.0.0.0:25565"],
            &[],
        );
        assert_eq!(c.listen.len(), 1);

        let c = cfg(&["--listen", "[::]:25565"], &[]);
        assert!(c.listen[0].is_ipv6());
    }

    #[test]
    fn env_fallbacks_and_precedence() {
        let c = cfg(&[], &[("RUTA_LISTEN", "0.0.0.0:25565,0.0.0.0:443")]);
        assert_eq!(c.listen.len(), 2);

        let c = cfg(
            &["--listen", "127.0.0.1:9"],
            &[("RUTA_LISTEN", "0.0.0.0:443")],
        );
        assert_eq!(c.listen, vec!["127.0.0.1:9".parse().unwrap()]);

        let c = cfg(
            &["--global-max", "10"],
            &[("RUTA_GLOBAL_MAX", "20"), ("RUTA_IDLE_TIMEOUT", "42")],
        );
        assert_eq!(c.global_max, 10);
        assert_eq!(c.idle_timeout, Duration::from_secs(42));
    }

    #[test]
    fn parses_sizes_and_durations() {
        assert_eq!(parse_bytes("8GiB").unwrap(), 8 * 1024 * 1024 * 1024);
        assert_eq!(parse_bytes("8g").unwrap(), 8 * 1024 * 1024 * 1024);
        assert_eq!(parse_bytes("512M").unwrap(), 512 * 1024 * 1024);
        assert_eq!(parse_bytes("1024").unwrap(), 1024);
        assert_eq!(parse_bytes("1k").unwrap(), 1024);
        assert!(parse_bytes("x").is_err());

        assert_eq!(parse_duration("300").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("300s").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_args(&["--listen", "example.com:25565"], &[]).is_err());
        assert!(parse_args(&["--listen", "0.0.0.0"], &[]).is_err());
        assert!(parse_args(&["--global-max", "0"], &[]).is_err());
        assert!(parse_args(&["--bogus"], &[]).is_err());
        assert!(parse_args(&["--per-sub-rate"], &[]).is_err());
        assert!(parse_args(&["--workers", "0"], &[]).is_err());
        assert!(parse_args(&[], &[("RUTA_GLOBAL_MAX", "nope")]).is_err());
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_args(&["--help"], &[]).unwrap(), Command::Help);
        assert_eq!(parse_args(&["-h"], &[]).unwrap(), Command::Help);
        assert_eq!(parse_args(&["-V"], &[]).unwrap(), Command::Version);
    }

    #[test]
    fn loopback_flag() {
        assert!(cfg(&["--allow-loopback-destinations"], &[]).allow_loopback_destinations);
        assert!(
            cfg(&[], &[("RUTA_ALLOW_LOOPBACK_DESTINATIONS", "true")]).allow_loopback_destinations
        );
        assert!(!cfg(&["--allow-loopback-destinations=0"], &[]).allow_loopback_destinations);
        // Command line wins over the environment in both directions.
        assert!(
            !cfg(
                &["--allow-loopback-destinations=0"],
                &[("RUTA_ALLOW_LOOPBACK_DESTINATIONS", "true")]
            )
            .allow_loopback_destinations
        );
        assert!(parse_args(&["--allow-loopback-destinations=maybe"], &[]).is_err());
    }
}
