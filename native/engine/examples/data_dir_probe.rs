//! On-disk compatibility probe: open an existing data dir (for example one
//! written by the JS engine) and print what the Rust engine reads from it.
//!
//! ```sh
//! cargo run -p espectral-engine --example data_dir_probe -- <data-dir>          # read only
//! cargo run -p espectral-engine --example data_dir_probe -- <data-dir> --write  # also write
//! ```
//!
//! `--write` performs a few writes through the Rust facade (offline account,
//! active account, instance patch, servers.dat, theme) so the JS engine can be
//! pointed at the same dir afterwards to confirm it reads them back.
//! No onboarding and no background tasks run, so nothing touches the network.

use espectral_engine::{Engine, EngineOptions};
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: data_dir_probe <data-dir> [--write]");
        std::process::exit(2);
    };
    let write = args.any(|a| a == "--write");
    let engine = Engine::new(EngineOptions {
        data_dir: Some(dir.into()),
        run_onboarding: false,
        start_background_tasks: false,
        ..Default::default()
    })
    .expect("engine");

    if write {
        engine.create_account("RustUser").await.expect("create_account");
        engine.set_active_account("RustUser").await.expect("set_active_account");
        let patch: espectral_engine::types::InstancePatch =
            serde_json::from_value(json!({ "memory_mb": 5120 })).expect("patch");
        engine.patch_instance("Parity Test", patch).await.expect("patch_instance");
        let servers: Vec<espectral_engine::types::ServerEntry> = serde_json::from_value(json!([
            { "name": "Local", "ip": "127.0.0.1:25565" },
            { "name": "Rust", "ip": "rust.example:25566" }
        ]))
        .expect("servers");
        engine.put_instance_servers("Parity Test", servers).await.expect("put_instance_servers");
        engine.set_theme("system").await.expect("set_theme");
    }

    let instances = engine.list_instances().await.expect("list_instances");
    let mut servers = serde_json::Map::new();
    for i in &instances {
        let s = engine.instance_servers(&i.name).await.expect("instance_servers");
        servers.insert(i.name.clone(), serde_json::to_value(s).unwrap_or(Value::Null));
    }
    let out = json!({
        "instances": instances,
        "accounts": engine.list_accounts().await.expect("list_accounts"),
        "servers": servers,
        "theme": engine.get_theme().await.expect("get_theme"),
        "accent": engine.get_accent().await.expect("get_accent"),
        "config": engine.get_config().await.expect("get_config"),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}
