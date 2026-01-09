//! Network and WiFi management task
//! Handles WiFi connection, network stack, and image downloads

use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_net::{Config, StackResources};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_24, PIN_25, PIN_29, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use crate::event::{Event, send_event};
use crate::network::{IMAGE_BUFFER_SIZE, download_image};
use crate::state::get_state;

/// Signal for triggering network update
static NETWORK_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for triggering LED blink
static LED_BLINK_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the network task to start update
pub fn signal_network_update() {
    NETWORK_UPDATE_SIGNAL.signal(());
}

/// Signals the network task to blink LED
pub fn signal_led_blink() {
    LED_BLINK_SIGNAL.signal(());
}

/// WiFi peripherals needed for initialization
pub struct WifiPeripherals {
    pub pwr_pin: embassy_rp::Peri<'static, PIN_23>,
    pub cs_pin: embassy_rp::Peri<'static, PIN_25>,
    pub pio: embassy_rp::Peri<'static, PIO0>,
    pub dio_pin: embassy_rp::Peri<'static, PIN_24>,
    pub clk_pin: embassy_rp::Peri<'static, PIN_29>,
    pub dma_ch: embassy_rp::Peri<'static, DMA_CH0>,
}

/// CYW43 runner task
#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

/// Network stack runner task
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

/// Network manager task - handles WiFi connection and network operations
#[embassy_executor::task]
pub async fn network_manager(
    spawner: Spawner,
    peripherals: WifiPeripherals,
    image_buffer: &'static mut [u8; IMAGE_BUFFER_SIZE],
) -> ! {
    info!("Network manager task started");
    Timer::after(Duration::from_secs(1)).await;

    // Load CYW43 firmware
    info!("Loading CYW43 firmware...");
    let fw = include_bytes!("../../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../../cyw43-firmware/43439A0_clm.bin");

    // Setup PIO for CYW43 SPI
    info!("Setting up PIO for CYW43 SPI...");
    let pwr = Output::new(peripherals.pwr_pin, Level::Low);
    let cs = Output::new(peripherals.cs_pin, Level::High);

    // Bind interrupts for PIO
    embassy_rp::bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    });

    let mut pio = Pio::new(peripherals.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        peripherals.dio_pin,
        peripherals.clk_pin,
        peripherals.dma_ch,
    );

    info!("Initializing CYW43 driver...");
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    info!("Spawning CYW43 runner task...");
    spawner.spawn(cyw43_task(runner)).unwrap();

    info!("Initializing CYW43 with CLM data...");
    control.init(clm).await;
    info!("Setting power management mode...");
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;
    info!("WiFi chip initialized successfully");

    // Init network stack
    info!("Initializing network stack...");
    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();

    // Generate pseudo-random seed from current time
    let seed = Instant::now().as_micros();
    info!("Network stack seed: {}", seed);

    let (stack, runner) = embassy_net::new(
        net_device,
        Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    info!("Spawning network stack runner task...");
    spawner.spawn(net_task(runner)).unwrap();

    // Main network loop - wait for signals from orchestrator
    info!("Network manager ready, waiting for signals...");
    loop {
        // Wait for either network update or LED blink signal
        let is_led_blink = match embassy_futures::select::select(
            async {
                NETWORK_UPDATE_SIGNAL.wait().await;
                false
            },
            async {
                LED_BLINK_SIGNAL.wait().await;
                true
            },
        )
        .await
        {
            embassy_futures::select::Either::First(val) => val,
            embassy_futures::select::Either::Second(val) => val,
        };

        if is_led_blink {
            info!("LED blink signal received");
            blink_led(&mut control).await;
            continue;
        }

        info!("Network update signal received, connecting to WiFi...");
        // Set performance mode for connection
        control
            .set_power_management(cyw43::PowerManagementMode::Performance)
            .await;

        // Connect to WiFi
        info!("Joining WiFi network: {}", crate::network::WIFI_SSID);
        while let Err(err) = control
            .join(
                crate::network::WIFI_SSID,
                JoinOptions::new(crate::network::WIFI_PASSWORD.as_bytes()),
            )
            .await
        {
            warn!("WiFi join failed: {:?}, retrying...", err.status);
            Timer::after(Duration::from_secs(1)).await;
        }

        info!("WiFi connected, waiting for link...");
        stack.wait_link_up().await;

        info!("Waiting for DHCP...");
        stack.wait_config_up().await;

        info!("Network stack is up!");
        if let Some(config) = stack.config_v4() {
            info!("IP address: {}", config.address);
        }

        // Update state
        {
            let mut state = get_state().await;
            state.wifi_connected = true;
        }
        send_event(Event::NetworkConnected).await;

        // Set WiFi to PowerSave mode
        control
            .set_power_management(cyw43::PowerManagementMode::PowerSave)
            .await;

        // Download image
        info!("Downloading image...");
        match download_image(&stack, image_buffer).await {
            Ok((image_data, server_delay)) => {
                info!("Image downloaded: {} bytes", image_data.len());

                // Update state with server delay if provided
                let delay_changed = {
                    let mut state = get_state().await;
                    let old_delay = state.next_update_delay_secs;

                    if let Some(delay) = server_delay {
                        state.next_update_delay_secs = delay;
                        info!("Next update will be in {} seconds (from server)", delay);
                    } else {
                        state.next_update_delay_secs =
                            crate::config::UPDATE_INTERVAL_MINUTES as u64 * 60;
                        info!(
                            "Next update will be in {} seconds (default)",
                            state.next_update_delay_secs
                        );
                    }
                    state.last_download_success = true;

                    // Check if delay changed
                    old_delay != state.next_update_delay_secs
                };

                send_event(Event::ImageDownloaded).await;

                // Notify scheduler if delay changed
                if delay_changed {
                    info!("Update delay changed, notifying scheduler");
                    send_event(Event::SchedulerUpdateRequested).await;
                }
            }
            Err(e) => {
                error!("Download failed: {}", e);

                // Update state
                {
                    let mut state = get_state().await;
                    state.last_download_success = false;
                }

                send_event(Event::ImageDownloadFailed).await;
            }
        }

        // Disconnect from WiFi properly
        info!("Disconnecting from WiFi...");
        disconnect_wifi(&mut control, &stack).await;

        // Update state
        {
            let mut state = get_state().await;
            state.wifi_connected = false;
        }
        send_event(Event::NetworkDisconnected).await;
    }
}

/// Disconnect from WiFi and wait for network stack to go down
async fn disconnect_wifi(control: &mut cyw43::Control<'static>, stack: &embassy_net::Stack<'_>) {
    control.leave().await;
    control.gpio_set(0, false).await;

    info!("Disconnected from WiFi");

    // Wait for network stack to go down
    info!("Waiting for network stack to go DOWN...");
    let mut timeout_counter = 0;
    while stack.is_link_up() || stack.is_config_up() {
        Timer::after(Duration::from_millis(100)).await;
        timeout_counter += 1;
        if timeout_counter > 50 {
            warn!("Timeout waiting for network stack to go down");
            break;
        }
    }
    info!("Network stack is DOWN");

    // Set to aggressive power management for maximum power savings
    control
        .set_power_management(cyw43::PowerManagementMode::SuperSave)
        .await;
}

/// Blink the onboard LED (controlled via CYW43)
async fn blink_led(control: &mut cyw43::Control<'_>) {
    info!("Blinking LED 5 times");
    for _ in 0..5 {
        control.gpio_set(0, true).await;
        Timer::after(Duration::from_millis(200)).await;
        control.gpio_set(0, false).await;
        Timer::after(Duration::from_millis(200)).await;
    }
    info!("LED blink complete");
}
