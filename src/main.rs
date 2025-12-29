#![no_std]
#![no_main]

use cyw43::JoinOptions;
use cyw43_pio::{PioSpi, DEFAULT_CLOCK_DIVIDER};
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::{Config, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

mod config;
mod epd_5in65f;
mod network;

use epd_5in65f::{Epd5in65f, EPD_5IN65F_WHITE};
use network::{download_image, wait_minutes, IMAGE_BUFFER_SIZE};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting e-Paper Weather Display");

    let p = embassy_rp::init(Default::default());

    // Init GPIOs for e-paper display (bit-banged SPI pins and keys)
    let (epd_pins, _keys) = config::init_all(p);

    // Init e-paper driver once
    let mut epd = Epd5in65f::new(epd_pins);

    // Load CYW43 firmware
    let fw = include_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../cyw43-firmware/43439A0_clm.bin");

    // Setup PIO for CYW43 SPI - steal peripherals for WiFi
    let p = unsafe { embassy_rp::Peripherals::steal() };
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(cyw43_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = Config::dhcpv4(Default::default());

    // Use a random seed
    let seed = 0x0123_4567_89AB_CDEFu64;

    // Init network stack
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );

    spawner.spawn(net_task(runner)).unwrap();

    // Main loop - update display periodically
    loop {
        // Set WiFi to PowerSave mode at the start of each cycle
        info!("Setting WiFi to PowerSave mode");
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;

        // Initialize e-paper panel before each update
        info!("EPD init");
        epd.init().await;

        // Connect to WiFi (re-connect each cycle)
        info!("Joining WiFi network: {}", network::WIFI_SSID);
        while let Err(err) = control
            .join(network::WIFI_SSID, JoinOptions::new(network::WIFI_PASSWORD.as_bytes()))
            .await
        {
            warn!("WiFi join failed: {:?}, retrying...", err.status);
            Timer::after(Duration::from_secs(1)).await;
        }

        info!("waiting for link...");
        stack.wait_link_up().await;

        info!("waiting for DHCP...");
        stack.wait_config_up().await;

        info!("Stack is up!");

        if let Some(config) = stack.config_v4() {
            info!("IP address: {}", config.address);
            if let Some(gateway) = config.gateway {
                info!("Gateway: {}", gateway);
            }
        }

        // Download and display image
        info!("Downloading image...");
        match download_image(&stack).await {
            Ok(image_data) => {
                // Validate image size before displaying
                if image_data.len() != IMAGE_BUFFER_SIZE {
                    error!(
                        "Invalid image size: got {} bytes, expected {} bytes. Skipping display.",
                        image_data.len(),
                        IMAGE_BUFFER_SIZE
                    );
                } else {
                    info!("Image downloaded: {} bytes", image_data.len());

                    // Clear display with white background
                    info!("Clear display");
                    epd.clear(EPD_5IN65F_WHITE).await;

                    // Display the downloaded image
                    info!("Display image data");
                    epd.display(image_data).await;
                }
            }
            Err(e) => {
                error!("Download failed: {}", e);
            }
        }

        // Put panel to sleep to save power
        info!("EPD sleep");
        epd.sleep().await;

        // Set WiFi to SuperSave mode for maximum power savings during sleep
        info!("Setting WiFi to SuperSave mode");
        control
            .set_power_management(cyw43::PowerManagementMode::SuperSave)
            .await;

        // Sleep until next update cycle
        info!("Sleeping for {} minutes until next update...", config::UPDATE_INTERVAL_MINUTES);
        wait_minutes(config::UPDATE_INTERVAL_MINUTES).await;
    }
}
