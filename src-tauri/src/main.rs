// EspectralClient Tauri shell — binary entry (thin wrapper over lib::run).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    espectral_client_lib::run();
}
