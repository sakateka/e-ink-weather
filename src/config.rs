//! GPIO configuration and helper initializers for the 5.65" e-Paper display.
//! Bit-banged SPI pins (CLK/MOSI) are provided via GPIOs.

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));

use embassy_rp::gpio::{Input, Level, Output, Pull};

/// Pins for e-Paper display (bit-banged SPI).
///
/// Mapping matches lib/config.c:
/// - RST  -> GPIO12
/// - DC   -> GPIO8
/// - CS   -> GPIO9
/// - BUSY -> GPIO13
/// - CLK  -> GPIO10
/// - MOSI -> GPIO11
pub struct EpdPins<'d> {
    pub rst: Output<'d>,
    pub dc: Output<'d>,
    pub cs: Output<'d>,
    pub busy: Input<'d>,
    pub clk: Output<'d>,
    pub mosi: Output<'d>,
}

/// Keys (buttons) per lib/epd_5in65f.h:
/// - KEY0 -> GPIO15
/// - KEY1 -> GPIO17
/// - KEY2 -> GPIO2
pub struct Keys<'d> {
    pub key0: Input<'d>,
    pub key1: Input<'d>,
    pub key2: Input<'d>,
}

/// Initialize all components (consumes Peripherals).
/// Returns bit-banged SPI GPIOs for the e-Paper and the three keys.
pub fn init_all(p: embassy_rp::Peripherals) -> (EpdPins<'static>, Keys<'static>) {
    // e-Paper control pins
    let rst = Output::new(p.PIN_12, Level::High);
    let dc = Output::new(p.PIN_8, Level::High);
    let cs = Output::new(p.PIN_9, Level::High);
    let busy = Input::new(p.PIN_13, Pull::Up);

    // Bit-banged SPI lines
    let clk = Output::new(p.PIN_10, Level::Low);
    let mosi = Output::new(p.PIN_11, Level::Low);

    let epd_pins = EpdPins { rst, dc, cs, busy, clk, mosi };

    // Keys
    let key0 = Input::new(p.PIN_15, Pull::Up);
    let key1 = Input::new(p.PIN_17, Pull::Up);
    let key2 = Input::new(p.PIN_2, Pull::Up);
    let keys = Keys { key0, key1, key2 };

    (epd_pins, keys)
}
