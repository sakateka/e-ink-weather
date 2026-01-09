//! # E-Paper Weather Display
//! Raspberry Pi Pico W based weather display using 5.65" e-Paper

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::adc::{Adc, Config as AdcConfig, InterruptHandler as AdcInterruptHandler};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::{ClockConfig, CoreVoltage};
use embassy_rp::config::Config;
use {defmt_rtt as _, panic_probe as _};

mod config;
mod epd_5in65f;
mod event;
mod network;
mod state;
mod task;

use network::IMAGE_BUFFER_SIZE;
use task::{
    WifiPeripherals, battery_monitor, button_handler, display_handler, network_manager,
    orchestrator, scheduler, wait_battery_ready,
};

/// Firmware version - automatically populated from Cargo.toml
pub static FIRMWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

// Static buffer for image data
static mut IMAGE_BUFFER: [u8; IMAGE_BUFFER_SIZE] = [0u8; IMAGE_BUFFER_SIZE];

bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => AdcInterruptHandler;
});

/// Helper function to spawn tasks and unwrap, panicking if spawn fails.
/// This is acceptable during initialization as we want to fail fast if we can't spawn a task.
#[allow(clippy::unwrap_used)]
fn spawn_unwrap<S>(spawner: &Spawner, token: embassy_executor::SpawnToken<S>) {
    spawner.spawn(token).unwrap();
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting e-Paper Weather Display v{}", FIRMWARE_VERSION);

    // Initialize the peripherals for the RP2040, use reduced clock settings for lower power consumption
    // Running at 5 MHz with 0.85V core voltage for minimal power consumption
    // This is sufficient for our low-frequency operations (button polling, periodic network updates)
    #[allow(clippy::unwrap_used)]
    let mut clock_config = ClockConfig::system_freq(5_000_000).unwrap();
    clock_config.core_voltage = CoreVoltage::V0_85;
    let config = Config::new(clock_config);
    let p = embassy_rp::init(config);

    // Setup ADC for battery voltage measurement
    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());

    // Spawn battery monitor task
    // Note: Uses GPIO28 (ADC2) with voltage divider (220Ω + 100Ω)
    // This does not conflict with WiFi pins
    spawn_unwrap(&spawner, battery_monitor(adc));

    // Wait for first battery measurement to complete
    info!("Waiting for initial battery measurement...");
    wait_battery_ready().await;
    info!("Battery measurement ready");

    // Initialize GPIO pins for e-paper display and buttons
    let (epd_pins, keys) = config::init_all(
        p.PIN_12, p.PIN_8, p.PIN_9, p.PIN_13, p.PIN_10, p.PIN_11, p.PIN_15, p.PIN_17, p.PIN_2,
    );

    // Spawn button handler task
    spawn_unwrap(&spawner, button_handler(keys));

    // Get static reference to image buffer
    // SAFETY: We're in single-threaded executor, buffer is only accessed by display and network tasks
    let image_buffer: &'static mut [u8; IMAGE_BUFFER_SIZE] =
        unsafe { &mut *core::ptr::addr_of_mut!(IMAGE_BUFFER) };

    // Split image buffer reference for display and network tasks
    // SAFETY: Display task only reads after network task writes, coordinated via events
    let display_buffer: &'static mut [u8; IMAGE_BUFFER_SIZE] =
        unsafe { &mut *(image_buffer as *mut _) };
    let network_buffer: &'static mut [u8; IMAGE_BUFFER_SIZE] =
        unsafe { &mut *(image_buffer as *mut _) };

    // Spawn display handler task
    spawn_unwrap(&spawner, display_handler(epd_pins, display_buffer));

    // Setup WiFi peripherals
    let wifi_peripherals = WifiPeripherals {
        pwr_pin: p.PIN_23,
        cs_pin: p.PIN_25,
        pio: p.PIO0,
        dio_pin: p.PIN_24,
        clk_pin: p.PIN_29,
        dma_ch: p.DMA_CH0,
    };

    // Spawn network manager task
    spawn_unwrap(
        &spawner,
        network_manager(spawner, wifi_peripherals, network_buffer),
    );

    // Spawn orchestrator tasks
    spawn_unwrap(&spawner, orchestrator());
    spawn_unwrap(&spawner, scheduler());

    info!("All tasks spawned successfully");
}
