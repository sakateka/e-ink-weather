//! Display management task
//! Handles e-Paper display updates and rendering

use defmt::{error, info};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

use crate::config::EpdPins;
use crate::epd_5in65f::{EPD_5IN65F_BLACK, EPD_5IN65F_WHITE, Epd5in65f, draw_number};
use crate::network::IMAGE_BUFFER_SIZE;
use crate::state::get_state;

/// Signal for triggering display update
static DISPLAY_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the display task to update
pub fn signal_display_update() {
    DISPLAY_UPDATE_SIGNAL.signal(());
}

/// Display handler task - manages e-Paper display updates
#[embassy_executor::task]
pub async fn display_handler(
    epd_pins: EpdPins<'static>,
    image_buffer: &'static mut [u8; IMAGE_BUFFER_SIZE],
) -> ! {
    info!("Display handler task started");

    // Initialize e-paper driver
    let mut epd = Epd5in65f::new(epd_pins);

    loop {
        // Wait for signal from orchestrator
        DISPLAY_UPDATE_SIGNAL.wait().await;

        info!("Display update signal received");

        // Get battery percentage from state
        let battery_percent = {
            let state = get_state().await;
            state.battery_percent
        };

        // Validate image size
        if image_buffer.len() != IMAGE_BUFFER_SIZE {
            error!(
                "Invalid image size: got {} bytes, expected {} bytes. Skipping display.",
                image_buffer.len(),
                IMAGE_BUFFER_SIZE
            );
            continue;
        }

        // Draw battery percentage in top-left corner
        info!("Drawing battery percentage: {}%", battery_percent);
        draw_number(image_buffer, 0, 0, battery_percent, EPD_5IN65F_BLACK, 2);

        // Initialize display
        info!("EPD init");
        epd.init().await;

        // Clear display with white background
        info!("Clear display");
        epd.clear(EPD_5IN65F_WHITE).await;

        // Display the image
        info!("Display image data");
        epd.display(image_buffer).await;

        // Put panel to sleep to save power
        info!("EPD sleep");
        epd.sleep().await;

        info!("Display update complete");
    }
}

/// Display test pattern (for debugging)
#[allow(dead_code)]
pub async fn display_test_pattern(epd: &mut Epd5in65f<'_>) {
    info!("Displaying test pattern");

    epd.init().await;
    epd.clear(EPD_5IN65F_WHITE).await;

    // Create a simple test pattern
    let mut test_buffer = [0x11u8; IMAGE_BUFFER_SIZE]; // White background

    // Draw some test content
    draw_number(&mut test_buffer, 10, 10, 42, EPD_5IN65F_BLACK, 3);

    epd.display(&test_buffer).await;
    epd.sleep().await;

    info!("Test pattern displayed");
}
