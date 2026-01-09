//! Event system for inter-task communication
//! Similar to pi-pico-alarmclock-rust event system

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

/// Maximum number of events that can be queued
const EVENT_QUEUE_SIZE: usize = 10;

/// Events that can be sent between tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Button 0 (KEY0) pressed - triggers immediate display refresh
    Key0Pressed,
    /// Button 1 (KEY1) pressed - triggers battery measurement
    Key1Pressed,
    /// Button 2 (KEY2) pressed - triggers LED blink
    Key2Pressed,
    /// Timer expired - triggers scheduled display refresh
    TimerExpired,
    /// Network connected
    NetworkConnected,
    /// Network disconnected
    NetworkDisconnected,
    /// Image downloaded successfully
    ImageDownloaded,
    /// Image download failed
    ImageDownloadFailed,
    /// Scheduler update requested - notifies scheduler that next_update_delay_secs has changed
    SchedulerUpdateRequested,
}

/// Global event channel for inter-task communication
pub static EVENT_CHANNEL: Channel<CriticalSectionRawMutex, Event, EVENT_QUEUE_SIZE> =
    Channel::new();

/// Send an event to the event channel (async)
pub async fn send_event(event: Event) {
    EVENT_CHANNEL.sender().send(event).await;
}

/// Receive an event from the event channel (blocking)
pub async fn receive_event() -> Event {
    EVENT_CHANNEL.receiver().receive().await
}
