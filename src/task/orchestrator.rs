//! Orchestrator task
//! Coordinates events and manages the main application flow

use defmt::info;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

use crate::event::{Event, receive_event, send_event};
use crate::state::get_state;
use crate::task::display::signal_display_update;
use crate::task::network::{signal_led_blink, signal_network_update};
use crate::task::power::signal_battery_measure;

/// Signal for interrupting the scheduler when delay changes
static SCHEDULER_INTERRUPT_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the scheduler to restart with updated delay
pub fn signal_scheduler_update() {
    SCHEDULER_INTERRUPT_SIGNAL.signal(());
}

/// Main orchestrator task - coordinates application flow based on events
#[embassy_executor::task]
pub async fn orchestrator() -> ! {
    info!("Orchestrator task started");

    // Trigger initial network update to start the first cycle
    signal_network_update();

    loop {
        // Wait for events
        let event = receive_event().await;

        match event {
            Event::Key0Pressed => {
                info!("KEY0 pressed - triggering immediate display refresh");
                // Signal network task to download image
                signal_network_update();
            }
            Event::Key1Pressed => {
                info!("KEY1 pressed - triggering battery measurement");
                // Signal power task to measure battery immediately
                signal_battery_measure();
            }
            Event::Key2Pressed => {
                info!("KEY2 pressed - triggering LED blink");
                // Signal network task to blink LED
                signal_led_blink();
            }
            Event::TimerExpired => {
                info!("Timer expired - triggering scheduled display refresh");
                // Signal network task to download image
                signal_network_update();
            }
            Event::NetworkConnected => {
                info!("Network connected");
            }
            Event::NetworkDisconnected => {
                info!("Network disconnected");
            }
            Event::ImageDownloaded => {
                info!("Image downloaded successfully - signaling display update");
                // Signal display task to update screen
                signal_display_update();
            }
            Event::ImageDownloadFailed => {
                info!("Image download failed");
            }
            Event::SchedulerUpdateRequested => {
                info!("Scheduler update requested - interrupting scheduler");
                // Signal scheduler to restart with new delay
                signal_scheduler_update();
            }
        }
    }
}

/// Scheduler task - manages periodic display updates based on configured intervals
/// Can be interrupted when the update delay changes
#[embassy_executor::task]
pub async fn scheduler() -> ! {
    info!("Scheduler task started");

    loop {
        // Get next update delay from state
        let delay_secs = {
            let state = get_state().await;
            state.next_update_delay_secs
        };

        info!(
            "Scheduler: waiting {} seconds until next update",
            delay_secs
        );

        // Wait for either timer expiration or scheduler interrupt signal
        match embassy_futures::select::select(
            Timer::after(Duration::from_secs(delay_secs)),
            SCHEDULER_INTERRUPT_SIGNAL.wait(),
        )
        .await
        {
            embassy_futures::select::Either::First(_) => {
                // Timer expired normally
                info!("Scheduler: timer expired, sending event");
                send_event(Event::TimerExpired).await;
            }
            embassy_futures::select::Either::Second(_) => {
                // Scheduler was interrupted due to delay update
                info!("Scheduler: interrupted, restarting with new delay");
                // Loop will restart with new delay from state
            }
        }
    }
}
