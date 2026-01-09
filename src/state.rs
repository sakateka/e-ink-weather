//! Global state management
//! Provides thread-safe access to shared state across tasks

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

/// Shared application state
pub struct AppState {
    /// Next update delay in seconds (can be updated by server response)
    pub next_update_delay_secs: u64,
    /// Battery percentage (0-100)
    pub battery_percent: u8,
    /// Whether WiFi is connected
    pub wifi_connected: bool,
    /// Last image download success
    pub last_download_success: bool,
}

impl AppState {
    /// Create new application state with default values
    pub const fn new(default_update_interval_minutes: u32) -> Self {
        Self {
            next_update_delay_secs: default_update_interval_minutes as u64 * 60,
            battery_percent: 0,
            wifi_connected: false,
            last_download_success: false,
        }
    }
}

/// Global application state, protected by mutex
pub static APP_STATE: Mutex<CriticalSectionRawMutex, AppState> =
    Mutex::new(AppState::new(crate::config::UPDATE_INTERVAL_MINUTES));

/// Get a reference to the global application state
pub async fn get_state()
-> embassy_sync::mutex::MutexGuard<'static, CriticalSectionRawMutex, AppState> {
    APP_STATE.lock().await
}
