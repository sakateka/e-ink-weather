//! Power management task
//! Monitors battery voltage and manages power states
//!
//! Battery voltage measurement using GPIO28 (ADC2) with voltage divider:
//! - Hardware: 220Ω + 100Ω resistor divider with 100pF capacitor
//! - Divider ratio: (220 + 100) / 100 = 3.2
//! - This allows measuring up to ~10.5V on a 3.3V ADC
//! - GPIO28 does not conflict with WiFi pins, so no coordination needed

use defmt::{info, warn};
use embassy_rp::adc::{Adc, Channel};
use embassy_rp::gpio::Pull;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

use crate::state::get_state;

/// Signal for triggering on-demand battery measurement
static BATTERY_MEASURE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal that gets set after first battery measurement is complete
static BATTERY_READY_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Request an immediate battery measurement (triggered by Key1)
pub fn signal_battery_measure() {
    BATTERY_MEASURE_SIGNAL.signal(());
}

/// Wait for the first battery measurement to complete
pub async fn wait_battery_ready() {
    BATTERY_READY_SIGNAL.wait().await;
}

/// Battery voltage reader task - periodically measures battery voltage
#[embassy_executor::task]
pub async fn battery_monitor(mut adc: Adc<'static, embassy_rp::adc::Async>) -> ! {
    info!("Battery monitor task started (GPIO28/ADC2)");

    // Configure GPIO28 as ADC input (ADC channel 2)
    let pin_28 = unsafe { embassy_rp::peripherals::PIN_28::steal() };
    let mut adc_channel = Channel::new_pin(pin_28, Pull::None);

    // Perform initial measurement immediately
    info!("Performing initial battery measurement...");
    let battery_percent = measure_battery_percentage(&mut adc, &mut adc_channel).await;
    {
        let mut state = get_state().await;
        state.battery_percent = battery_percent;
    }
    info!("Initial battery: {}%", battery_percent);

    // Signal that first measurement is complete
    BATTERY_READY_SIGNAL.signal(());

    loop {
        // Wait for either periodic timer (5 minutes) or manual trigger signal
        embassy_futures::select::select(
            Timer::after(Duration::from_secs(300)),
            BATTERY_MEASURE_SIGNAL.wait(),
        )
        .await;

        // Measure battery voltage
        let battery_percent = measure_battery_percentage(&mut adc, &mut adc_channel).await;

        // Update state
        {
            let mut state = get_state().await;
            state.battery_percent = battery_percent;
        }

        info!("Battery: {}%", battery_percent);
    }
}

/// Measure battery voltage and convert to percentage
/// Uses median filtering to reject noise
///
/// Hardware setup:
/// - Battery voltage → 220Ω → GPIO28 → 100Ω → GND
/// - 100pF capacitor between GPIO28 and GND for noise filtering
/// - Voltage divider ratio: 3.2 (measures up to ~10.5V)
async fn measure_battery_percentage(
    adc: &mut Adc<'static, embassy_rp::adc::Async>,
    adc_channel: &mut Channel<'static>,
) -> u8 {
    const SAMPLE_COUNT: usize = 9;
    const SAMPLE_DELAY_MS: u64 = 5;

    let mut samples = [0u16; SAMPLE_COUNT];
    let mut valid_samples = 0;

    // Small delay to let the voltage stabilize
    Timer::after(Duration::from_micros(100)).await;

    // Collect ADC samples
    for i in 0..SAMPLE_COUNT {
        if let Ok(adc_value) = adc.read(adc_channel).await {
            samples[i] = adc_value;
            valid_samples += 1;
        }
        Timer::after(Duration::from_millis(SAMPLE_DELAY_MS)).await;
    }

    if valid_samples == 0 {
        warn!("No valid ADC samples, returning 0%");
        return 0;
    }

    // Sort samples to find median
    samples[..valid_samples].sort_unstable();
    let median_adc = samples[valid_samples / 2];

    // Convert ADC value to voltage
    // ADC reference voltage: 3.3V
    // ADC resolution: 12-bit (4096 levels)
    // Voltage divider ratio: (220 + 100) / 100 = 3.2
    let adc_voltage = f32::from(median_adc) * 3.3 / 4096.0;
    let battery_voltage = adc_voltage * 3.2;

    info!(
        "Battery voltage: {}V (ADC: {}, ADC voltage: {}V)",
        battery_voltage, median_adc, adc_voltage
    );

    // Convert voltage to percentage
    // LiPo battery: ~4.2V (100%) to ~3.0V (0%)
    // Using linear approximation
    let percentage = if battery_voltage >= 4.2 {
        100.0
    } else if battery_voltage <= 3.0 {
        0.0
    } else {
        ((battery_voltage - 3.0) / (4.2 - 3.0)) * 100.0
    };

    percentage.clamp(0.0, 100.0) as u8
}
