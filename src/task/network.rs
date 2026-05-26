//! Network and WiFi management task
//! Handles WiFi connection, network stack, and image downloads

use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use cortex_m::{interrupt, peripheral::SCB};
use defmt::{error, info, warn};
use embassy_futures::select::{Either, select};
use embassy_executor::Spawner;
use embassy_net::{Config, StackResources};
use embassy_rp::dma::{Channel, InterruptHandler as DmaInterruptHandler};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_24, PIN_25, PIN_29, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use static_cell::StaticCell;

use crate::event::{Event, send_event};
use crate::network::{IMAGE_BUFFER_SIZE, download_image};
use crate::state::get_state;
use crate::task::display::signal_display_update;

/// Signal for triggering network update
static NETWORK_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for triggering LED blink
static LED_BLINK_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Shared CYW43 control for LED blinking - will be initialized once
static CYW43_CONTROL: StaticCell<Mutex<CriticalSectionRawMutex, cyw43::Control<'static>>> =
    StaticCell::new();

/// Reference to the initialized control mutex (set after initialization)
static mut CYW43_CONTROL_REF: Option<
    &'static Mutex<CriticalSectionRawMutex, cyw43::Control<'static>>,
> = None;

/// Upper bounds to prevent waiting forever in bad network conditions.
const WIFI_JOIN_TOTAL_TIMEOUT_SECS: u64 = 90;
const WIFI_JOIN_MAX_RETRIES: u8 = 8;
const WIFI_JOIN_PROGRESS_LOG_SECS: u64 = 10;
/// If `join()` never returns (driver stuck), we cannot cancel the future safely — reset the MCU.
const WIFI_JOIN_STUCK_RESET_SECS: u64 = 90;
const WIFI_LINK_TIMEOUT_SECS: u64 = 20;
const DHCP_TIMEOUT_SECS: u64 = 20;
const HTTP_DOWNLOAD_TIMEOUT_SECS: u64 = 45;

/// Clamp dynamic server delay to safe bounds.
const MIN_NEXT_UPDATE_DELAY_SECS: u64 = 30;
const MAX_NEXT_UPDATE_DELAY_SECS: u64 = 12 * 60 * 60;
const WIFI_WARNING_RETRY_COUNT: u8 = 3;

fn sanitize_next_delay_secs(delay_secs: u64) -> u64 {
    delay_secs.clamp(MIN_NEXT_UPDATE_DELAY_SECS, MAX_NEXT_UPDATE_DELAY_SECS)
}

fn resolve_next_delay_secs(server_delay: Option<u64>) -> u64 {
    server_delay
        .map(sanitize_next_delay_secs)
        .unwrap_or_else(|| sanitize_next_delay_secs(crate::config::UPDATE_INTERVAL_MINUTES as u64 * 60))
}

async fn apply_next_delay(server_delay: Option<u64>) -> bool {
    let mut state = get_state().await;
    let old_delay = state.next_update_delay_secs;
    let new_delay = resolve_next_delay_secs(server_delay);

    if let Some(delay) = server_delay {
        if new_delay != delay {
            warn!(
                "Server delay {}s is out of bounds, clamped to {}s",
                delay, new_delay
            );
        }
        info!("Next update will be in {} seconds (from server)", new_delay);
    } else {
        info!("Next update will be in {} seconds (default)", new_delay);
    }

    state.next_update_delay_secs = new_delay;
    state.last_download_success = true;
    old_delay != new_delay
}

async fn mark_download_failed(wifi_issue: bool) {
    let mut state = get_state().await;
    state.last_download_success = false;
    if wifi_issue {
        state.wifi_retry_count = WIFI_WARNING_RETRY_COUNT;
    }
}

async fn fail_download_and_refresh(wifi_issue: bool) {
    mark_download_failed(wifi_issue).await;
    send_event(Event::ImageDownloadFailed).await;
}

#[derive(Clone, Copy)]
enum NetworkCycleState {
    JoinWifi,
    WaitNetworkReady,
    DownloadImage,
    FinalizeSuccess,
    FinalizeFailure { wifi_issue: bool },
    Disconnect,
}

async fn connect_wifi_with_retries(
    control_mutex: &Mutex<CriticalSectionRawMutex, cyw43::Control<'static>>,
) -> bool {
    info!("Joining WiFi network: {}", crate::network::WIFI_SSID);

    let mut join_retry_count: u8 = 0;
    let join_start = Instant::now();

    loop {
        if join_start.elapsed().as_secs() >= WIFI_JOIN_TOTAL_TIMEOUT_SECS {
            warn!(
                "WiFi join timed out after {}s total",
                WIFI_JOIN_TOTAL_TIMEOUT_SECS
            );
            return false;
        }

        let mut control = control_mutex.lock().await;

        control
            .set_power_management(cyw43::PowerManagementMode::Performance)
            .await;

        let join_result = {
            let attempt_started = Instant::now();
            let mut join_future = core::pin::pin!(control.join(
                crate::network::WIFI_SSID,
                JoinOptions::new(crate::network::WIFI_PASSWORD.as_bytes()),
            ));
            loop {
                match select(
                    join_future.as_mut(),
                    Timer::after(Duration::from_secs(WIFI_JOIN_PROGRESS_LOG_SECS)),
                )
                .await
                {
                    Either::First(result) => break result,
                    Either::Second(_) => {
                        let attempt_secs = attempt_started.elapsed().as_secs();
                        let cycle_secs = join_start.elapsed().as_secs();
                        warn!(
                            "WiFi join still pending ({}s this attempt, {}s total connect)",
                            attempt_secs, cycle_secs
                        );
                        if attempt_secs >= WIFI_JOIN_STUCK_RESET_SECS {
                            error!(
                                "WiFi join stuck > {}s (cannot cancel join safely); resetting MCU",
                                WIFI_JOIN_STUCK_RESET_SECS
                            );
                            interrupt::disable();
                            SCB::sys_reset();
                        }
                        if cycle_secs >= WIFI_JOIN_TOTAL_TIMEOUT_SECS {
                            error!(
                                "WiFi connect exceeded {}s total; resetting MCU",
                                WIFI_JOIN_TOTAL_TIMEOUT_SECS
                            );
                            interrupt::disable();
                            SCB::sys_reset();
                        }
                    }
                }
            }
        };

        match join_result {
            Ok(_) => return true,
            Err(err) => {
                warn!("WiFi join failed with error: {:?}, retrying...", err);
            }
        }

        join_retry_count = join_retry_count.saturating_add(1);
        {
            let mut state = get_state().await;
            state.wifi_retry_count = join_retry_count;
        }

        if join_retry_count >= WIFI_JOIN_MAX_RETRIES {
            warn!("WiFi join reached retry limit: {}", WIFI_JOIN_MAX_RETRIES);
            return false;
        }

        drop(control);
        Timer::after(Duration::from_secs(1)).await;
    }
}

async fn wait_network_ready(stack: &embassy_net::Stack<'_>) -> bool {
    info!("WiFi connected, waiting for link...");
    if with_timeout(
        Duration::from_secs(WIFI_LINK_TIMEOUT_SECS),
        stack.wait_link_up(),
    )
    .await
    .is_err()
    {
        warn!(
            "Timeout waiting for link up ({}s)",
            WIFI_LINK_TIMEOUT_SECS
        );
        return false;
    }

    info!("Waiting for DHCP...");
    if with_timeout(
        Duration::from_secs(DHCP_TIMEOUT_SECS),
        stack.wait_config_up(),
    )
    .await
    .is_err()
    {
        warn!("Timeout waiting for DHCP config ({}s)", DHCP_TIMEOUT_SECS);
        return false;
    }

    true
}

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
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
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
    const FW_LEN: usize = include_bytes!("../../cyw43-firmware/43439A0.bin").len();
    const CLM_LEN: usize = include_bytes!("../../cyw43-firmware/43439A0_clm.bin").len();
    const NVRAM_LEN: usize = include_bytes!("../../cyw43-firmware/nvram_rp2040.bin").len();
    static FW: cyw43::Aligned<cyw43::A4, [u8; FW_LEN]> =
        cyw43::Aligned(*include_bytes!("../../cyw43-firmware/43439A0.bin"));
    static CLM: cyw43::Aligned<cyw43::A4, [u8; CLM_LEN]> =
        cyw43::Aligned(*include_bytes!("../../cyw43-firmware/43439A0_clm.bin"));
    static NVRAM: cyw43::Aligned<cyw43::A4, [u8; NVRAM_LEN]> =
        cyw43::Aligned(*include_bytes!("../../cyw43-firmware/nvram_rp2040.bin"));

    // Setup PIO for CYW43 SPI
    info!("Setting up PIO for CYW43 SPI...");
    let pwr = Output::new(peripherals.pwr_pin, Level::Low);
    let cs = Output::new(peripherals.cs_pin, Level::High);

    // Bind interrupts for PIO
    embassy_rp::bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
        DMA_IRQ_0 => DmaInterruptHandler<DMA_CH0>;
    });

    let mut pio = Pio::new(peripherals.pio, Irqs);
    let dma = Channel::new(peripherals.dma_ch, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        peripherals.dio_pin,
        peripherals.clk_pin,
        dma,
    );

    info!("Initializing CYW43 driver...");
    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, &FW, &NVRAM).await;

    info!("Spawning CYW43 runner task...");
    #[allow(clippy::unwrap_used)]
    spawner.spawn(cyw43_task(runner).unwrap());

    info!("Initializing CYW43 with CLM data...");
    control.init(&CLM[..]).await;

    control.gpio_set(0, true).await;
    info!("Initial LED turned ON");

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

    // Configure DHCP with hostname
    let mut dhcp_config = embassy_net::DhcpConfig::default();
    let s: heapless::String<32> = heapless::String::try_from("weather").unwrap();
    dhcp_config.hostname = Some(s);

    let (stack, runner) = embassy_net::new(
        net_device,
        Config::dhcpv4(dhcp_config),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    info!("Network hostname set to: weather");

    info!("Spawning network stack runner task...");
    #[allow(clippy::unwrap_used)]
    spawner.spawn(net_task(runner).unwrap());

    // Store control in static mutex for sharing
    let control_mutex = CYW43_CONTROL.init(Mutex::new(control));

    // Save reference for other tasks
    unsafe {
        CYW43_CONTROL_REF = Some(control_mutex);
    }

    // Spawn LED blink task
    info!("Spawning LED blink task...");
    #[allow(clippy::unwrap_used)]
    spawner.spawn(led_blink_task().unwrap());

    // Track if initial LED is still on
    let mut initial_led_on = true;

    // Main network loop - wait for signals from orchestrator
    info!("Network manager ready, waiting for signals...");
    loop {
        // Wait for network update signal
        NETWORK_UPDATE_SIGNAL.wait().await;

        info!("Network update signal received");
        {
            let mut state = get_state().await;
            state.wifi_retry_count = 0;
        }

        let mut cycle_state = NetworkCycleState::JoinWifi;
        let mut delay_changed = false;
        while !matches!(cycle_state, NetworkCycleState::Disconnect) {
            cycle_state = match cycle_state {
                NetworkCycleState::JoinWifi => {
                    if connect_wifi_with_retries(control_mutex).await {
                        NetworkCycleState::WaitNetworkReady
                    } else {
                        NetworkCycleState::FinalizeFailure { wifi_issue: true }
                    }
                }
                NetworkCycleState::WaitNetworkReady => {
                    if wait_network_ready(&stack).await {
                        info!("Network stack is up!");
                        if let Some(config) = stack.config_v4() {
                            info!("IP address: {}", config.address);
                        }
                        {
                            let mut state = get_state().await;
                            state.wifi_connected = true;
                            state.wifi_retry_count = 0;
                        }
                        send_event(Event::NetworkConnected).await;
                        NetworkCycleState::DownloadImage
                    } else {
                        NetworkCycleState::FinalizeFailure { wifi_issue: true }
                    }
                }
                NetworkCycleState::DownloadImage => {
                    info!("Downloading image...");
                    match with_timeout(
                        Duration::from_secs(HTTP_DOWNLOAD_TIMEOUT_SECS),
                        download_image(&stack, image_buffer),
                    )
                    .await
                    {
                        Ok(Ok((image_data, server_delay))) => {
                            info!("Image downloaded: {} bytes", image_data.len());
                            delay_changed = apply_next_delay(server_delay).await;
                            NetworkCycleState::FinalizeSuccess
                        }
                        Ok(Err(e)) => {
                            error!("Download failed: {}", e);
                            NetworkCycleState::FinalizeFailure { wifi_issue: true }
                        }
                        Err(_) => {
                            error!(
                                "Image download timed out after {} seconds",
                                HTTP_DOWNLOAD_TIMEOUT_SECS
                            );
                            NetworkCycleState::FinalizeFailure { wifi_issue: true }
                        }
                    }
                }
                NetworkCycleState::FinalizeSuccess => {
                    send_event(Event::ImageDownloaded).await;
                    if delay_changed {
                        info!("Update delay changed, notifying scheduler");
                        send_event(Event::SchedulerUpdateRequested).await;
                    }
                    NetworkCycleState::Disconnect
                }
                NetworkCycleState::FinalizeFailure { wifi_issue } => {
                    {
                        let mut state = get_state().await;
                        state.wifi_connected = false;
                    }
                    fail_download_and_refresh(wifi_issue).await;
                    if wifi_issue {
                        // Trigger a single final render for this cycle.
                        signal_display_update();
                    }
                    NetworkCycleState::Disconnect
                }
                NetworkCycleState::Disconnect => NetworkCycleState::Disconnect,
            };
        }

        {
            // Set WiFi to PowerSave mode
            let mut control = control_mutex.lock().await;
            control
                .set_power_management(cyw43::PowerManagementMode::PowerSave)
                .await;

            // Disconnect from WiFi properly
            info!("Disconnecting from WiFi...");
            disconnect_wifi(&mut control, &stack).await;

            // Turn off initial LED if it's still on
            if initial_led_on {
                control.gpio_set(0, false).await;
                info!("Initial LED turned OFF");
                initial_led_on = false;
            }
        }

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

/// LED blink task - handles LED blinking on demand (e.g., KEY2 press)
#[embassy_executor::task]
pub async fn led_blink_task() -> ! {
    info!("LED blink task started");

    loop {
        // Wait for LED blink signal
        LED_BLINK_SIGNAL.wait().await;
        info!("LED blink signal received");

        // Get control mutex reference
        let control_mutex = unsafe { CYW43_CONTROL_REF };

        if control_mutex.is_none() {
            warn!("CYW43_CONTROL not initialized yet, skipping LED blink");
            continue;
        }

        let control_mutex = control_mutex.unwrap();
        let mut control = control_mutex.lock().await;

        for _ in 0..5 {
            control.gpio_set(0, true).await;
            Timer::after(Duration::from_millis(200)).await;
            control.gpio_set(0, false).await;
            Timer::after(Duration::from_millis(200)).await;
        }
        info!("LED blink complete");
    }
}
