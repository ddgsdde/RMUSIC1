#[cfg(feature = "tauri-app")]
pub mod app;
#[cfg(feature = "tauri-app")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    app::run();
}
pub mod cache;
pub mod mobile_quality;
pub mod core_artwork;
pub mod download_contract;
mod cache_tail;
pub mod network;
pub mod probe;
pub mod profiles;
pub mod offline_stats;
pub mod group_output;
