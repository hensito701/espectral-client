//! `ruta-relay` binary.

use std::process::ExitCode;

use ruta_relay::config::{self, Command};
use ruta_relay::server::Server;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match config::parse(args, |k| std::env::var(k).ok()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ruta-relay: {e}");
            return ExitCode::from(2);
        }
    };

    let cfg = match cmd {
        Command::Help => {
            print!("{}", config::USAGE);
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("ruta-relay {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Run(cfg) => cfg,
    };

    let workers = cfg.workers;
    let server = match make_server(cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ruta-relay: {e}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .thread_name("ruta")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ruta-relay: failed to start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(async move { server.run().await }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ruta-relay: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Production builds always use the pinned key.
#[cfg(not(feature = "test-key"))]
fn make_server(cfg: ruta_relay::Config) -> Result<Server, String> {
    Ok(Server::new(cfg))
}

/// `test-key` builds (never shipped) may swap in a throwaway Ed25519 key from
/// `RUTA_TEST_KEY_SEED` (64 hex chars) so a smoke run can mint its own grant.
#[cfg(feature = "test-key")]
fn make_server(cfg: ruta_relay::Config) -> Result<Server, String> {
    match std::env::var("RUTA_TEST_KEY_SEED") {
        Ok(hex) if !hex.trim().is_empty() => {
            let raw = hex.trim();
            let mut seed = [0u8; 32];
            if raw.len() != 64 {
                return Err("RUTA_TEST_KEY_SEED must be 64 hex characters".into());
            }
            for (i, byte) in seed.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16)
                    .map_err(|_| "RUTA_TEST_KEY_SEED must be 64 hex characters")?;
            }
            let key = ed25519_dalek::SigningKey::from_bytes(&seed);
            Ok(Server::new_with_verifier(
                cfg,
                ruta_relay::Verifier::with_key(key.verifying_key()),
            ))
        }
        _ => Ok(Server::new(cfg)),
    }
}
