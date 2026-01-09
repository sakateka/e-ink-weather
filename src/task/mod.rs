//! Task modules
//! Contains all background tasks for the application

pub mod buttons;
pub mod display;
pub mod network;
pub mod orchestrator;
pub mod power;

// Re-export commonly used items
pub use buttons::button_handler;
pub use display::display_handler;
pub use network::{WifiPeripherals, network_manager};
pub use orchestrator::{orchestrator, scheduler};
pub use power::{battery_monitor, wait_battery_ready};
