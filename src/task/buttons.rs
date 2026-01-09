//! Button handling task
//! Monitors button presses and sends events

use defmt::info;
use embassy_futures::select::select3;
use embassy_time::{Duration, Timer};

use crate::config::Keys;
use crate::event::{Event, send_event};

/// Button identifiers
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Key0,
    Key1,
    Key2,
}

impl defmt::Format for Button {
    fn format(&self, f: defmt::Formatter) {
        match self {
            Button::Key0 => defmt::write!(f, "Key0"),
            Button::Key1 => defmt::write!(f, "Key1"),
            Button::Key2 => defmt::write!(f, "Key2"),
        }
    }
}

/// Button handler task - monitors all buttons and sends events
#[embassy_executor::task]
pub async fn button_handler(mut keys: Keys<'static>) -> ! {
    info!("Button handler task started");

    loop {
        // Wait for any button press using GPIO interrupts
        let button = wait_for_button_press(&mut keys).await;

        // Send appropriate event
        let event = match button {
            Button::Key0 => Event::Key0Pressed,
            Button::Key1 => Event::Key1Pressed,
            Button::Key2 => Event::Key2Pressed,
        };

        info!("Button {:?} pressed, sending event", button);
        send_event(event).await;
    }
}

/// Wait for any button press using GPIO interrupts (efficient, no polling).
/// Returns button identifier.
/// Buttons are active-low with pull-up resistors.
async fn wait_for_button_press(keys: &mut Keys<'_>) -> Button {
    loop {
        // Wait for any button to be pressed (falling edge = button pressed)
        // This uses GPIO interrupts - CPU can sleep until interrupt occurs
        let button = select3(
            keys.key0.wait_for_falling_edge(),
            keys.key1.wait_for_falling_edge(),
            keys.key2.wait_for_falling_edge(),
        )
        .await;

        // Debounce delay
        Timer::after(Duration::from_millis(50)).await;

        // Verify button is still pressed and wait for release
        match button {
            embassy_futures::select::Either3::First(_) => {
                if keys.key0.is_low() {
                    // Wait for button release (rising edge)
                    keys.key0.wait_for_rising_edge().await;
                    return Button::Key0;
                }
            }
            embassy_futures::select::Either3::Second(_) => {
                if keys.key1.is_low() {
                    keys.key1.wait_for_rising_edge().await;
                    return Button::Key1;
                }
            }
            embassy_futures::select::Either3::Third(_) => {
                if keys.key2.is_low() {
                    keys.key2.wait_for_rising_edge().await;
                    return Button::Key2;
                }
            }
        }
        // If debounce failed, loop and wait for next press
    }
}
