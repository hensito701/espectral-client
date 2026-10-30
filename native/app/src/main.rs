//! espectral-client — native launcher binary.
//!
//! Phase 1 (engine port): constructs the engine (data dir resolution,
//! onboarding, cluster start hooks) and prints `health()` as JSON. The Slint
//! UI replaces this body in a later phase.

use espectral_engine::{Engine, EngineOptions};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("ESPECTRAL_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();

    let engine = match Engine::start(EngineOptions::default()).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine failed to start: {e}");
            std::process::exit(1);
        }
    };
    match engine.health().await {
        Ok(h) => println!("{}", serde_json::to_string(&h).unwrap_or_default()),
        Err(e) => eprintln!("health failed: {e}"),
    }
    engine.shutdown().await;
}
