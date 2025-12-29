//! Driver for 5.65 inch e-Paper display (600x448 pixels)
//! Bit-banged SPI over GPIO, aligned with Waveshare C reference.

use embassy_time::{Duration, Timer};

use crate::config::EpdPins;

/// Display dimensions
pub const EPD_5IN65F_WIDTH: u16 = 600;
pub const EPD_5IN65F_HEIGHT: u16 = 448;

/// Colors: 3-bit indices matching lib/epd_5in65f.h
// pub const EPD_5IN65F_BLACK: u8 = 0x0;
pub const EPD_5IN65F_WHITE: u8 = 0x1;
/*
pub const EPD_5IN65F_GREEN: u8 = 0x2;
pub const EPD_5IN65F_BLUE: u8 = 0x3;
pub const EPD_5IN65F_RED: u8 = 0x4;
pub const EPD_5IN65F_YELLOW: u8 = 0x5;
pub const EPD_5IN65F_ORANGE: u8 = 0x6;
pub const EPD_5IN65F_CLEAN: u8 = 0x7;
*/

/// e-Paper driver structure
pub struct Epd5in65f<'d> {
    pins: EpdPins<'d>,
}

impl<'d> Epd5in65f<'d> {
    /// Create new driver instance
    pub fn new(pins: EpdPins<'d>) -> Self {
        Self { pins }
    }

    /// Software reset (EPD_RST high->low->high with delays)
    async fn reset(&mut self) {
        self.pins.rst.set_high();
        Timer::after(Duration::from_millis(200)).await;
        self.pins.rst.set_low();
        Timer::after(Duration::from_millis(2)).await;
        self.pins.rst.set_high();
        Timer::after(Duration::from_millis(200)).await;
    }

    /// Bit-banged SPI: write single byte, MSB first
    fn spi_write_byte(&mut self, mut value: u8) {
        for _ in 0..8 {
            self.pins.clk.set_low();
            if (value & 0x80) != 0 {
                self.pins.mosi.set_high();
            } else {
                self.pins.mosi.set_low();
            }
            self.pins.clk.set_high();
            value <<= 1;
        }
        self.pins.clk.set_low();
    }

    /// Send command
    fn send_command(&mut self, reg: u8) {
        self.pins.dc.set_low();
        self.pins.cs.set_low();
        self.spi_write_byte(reg);
        self.pins.cs.set_high();
    }

    /// Send data byte
    fn send_data(&mut self, data: u8) {
        self.pins.dc.set_high();
        self.pins.cs.set_low();
        self.spi_write_byte(data);
        self.pins.cs.set_high();
    }

    /*
    /// Send data buffer
    fn send_data_buffer(&mut self, data: &[u8]) {
        for &b in data {
            self.send_data(b);
        }
    }
    */

    /// Wait until BUSY becomes high
    async fn wait_busy_high(&mut self) {
        defmt::debug!("wait_busy_high: starting, current state={}", self.pins.busy.is_high());
        let mut iterations = 0u32;
        while !self.pins.busy.is_high() {
            Timer::after(Duration::from_millis(1)).await;
            iterations += 1;
            if iterations & 127 == 0 {
                defmt::debug!("wait_busy_high: still waiting, iterations={}, state={}", iterations, self.pins.busy.is_high());
            }
        }
        defmt::debug!("wait_busy_high: done after {} iterations, final state={}", iterations, self.pins.busy.is_high());
    }

    /// Wait until BUSY becomes low
    async fn wait_busy_low(&mut self) {
        defmt::debug!("wait_busy_low: starting, current state={}", self.pins.busy.is_high());
        let mut iterations = 0u32;
        while self.pins.busy.is_high() {
            Timer::after(Duration::from_millis(1)).await;
            iterations += 1;
            if iterations & 127 == 0 {
                defmt::debug!("wait_busy_low: still waiting, iterations={}, state={}", iterations, self.pins.busy.is_high());
            }
        }
        defmt::debug!("wait_busy_low: done after {} iterations, final state={}", iterations, self.pins.busy.is_high());
    }

    /// Initialize display (sequence mirrors C)
    pub async fn init(&mut self) {
        self.reset().await;
        self.wait_busy_high().await;

        self.send_command(0x00);
        self.send_data(0xEF);
        self.send_data(0x08);

        self.send_command(0x01);
        self.send_data(0x37);
        self.send_data(0x00);
        self.send_data(0x23);
        self.send_data(0x23);

        self.send_command(0x03);
        self.send_data(0x00);

        self.send_command(0x06);
        self.send_data(0xC7);
        self.send_data(0xC7);
        self.send_data(0x1D);

        self.send_command(0x30);
        self.send_data(0x3C);

        self.send_command(0x41);
        self.send_data(0x00);

        self.send_command(0x50);
        self.send_data(0x37);

        self.send_command(0x60);
        self.send_data(0x22);

        self.send_command(0x61);
        self.send_data(0x02);
        self.send_data(0x58);
        self.send_data(0x01);
        self.send_data(0xC0);

        self.send_command(0xE3);
        self.send_data(0xAA);

        Timer::after(Duration::from_millis(100)).await;

        self.send_command(0x50);
        self.send_data(0x37);
    }

    /// Clear screen to given 3-bit color index
    pub async fn clear(&mut self, color: u8) {
        self.send_command(0x61); // Set Resolution
        self.send_data(0x02);
        self.send_data(0x58);
        self.send_data(0x01);
        self.send_data(0xC0);

        self.send_command(0x10);

        // Each byte is two pixels: high nibble and low nibble
        let width_half = EPD_5IN65F_WIDTH / 2;
        let byte = ((color & 0x0F) << 4) | (color & 0x0F);

        for _y in 0..EPD_5IN65F_HEIGHT {
            for _x in 0..width_half {
                self.send_data(byte);
            }
        }

        self.send_command(0x04);
        self.wait_busy_high().await;
        self.send_command(0x12);
        self.wait_busy_high().await;
        self.send_command(0x02);
        self.wait_busy_low().await;
        Timer::after(Duration::from_millis(500)).await;
    }

    /// Display image buffer, 4bpp packed (two pixels per byte), row-major
    pub async fn display(&mut self, image: &[u8]) {
        self.send_command(0x61); // Set Resolution
        self.send_data(0x02);
        self.send_data(0x58);
        self.send_data(0x01);
        self.send_data(0xC0);

        self.send_command(0x10);

        let width_half = EPD_5IN65F_WIDTH / 2;
        for i in 0..EPD_5IN65F_HEIGHT as usize {
            for j in 0..width_half as usize {
                let idx = j + (width_half as usize * i);
                let b = image.get(idx).copied().unwrap_or(0x11);
                self.send_data(b);
            }
        }

        self.send_command(0x04);
        self.wait_busy_high().await;
        self.send_command(0x12);
        self.wait_busy_high().await;
        self.send_command(0x02);
        self.wait_busy_low().await;
        Timer::after(Duration::from_millis(200)).await;
    }

    /*
    /// Display sub-rectangle from image buffer at (xstart, ystart)
    pub async fn display_part(
        &mut self,
        image: &[u8],
        xstart: u16,
        ystart: u16,
        image_width: u16,
        image_height: u16,
    ) {
        self.send_command(0x61); // Set Resolution
        self.send_data(0x02);
        self.send_data(0x58);
        self.send_data(0x01);
        self.send_data(0xC0);

        self.send_command(0x10);

        let width_half = EPD_5IN65F_WIDTH / 2;
        for i in 0..EPD_5IN65F_HEIGHT {
            for j in 0..width_half {
                if i < image_height + ystart
                    && i >= ystart
                    && j < (image_width + xstart) / 2
                    && j >= xstart / 2
                {
                    let idx = ((j - xstart / 2) + (image_width / 2 * (i - ystart))) as usize;
                    let b = image.get(idx).copied().unwrap_or(0x11);
                    self.send_data(b);
                } else {
                    self.send_data(0x11);
                }
            }
        }

        self.send_command(0x04);
        self.wait_busy_high().await;
        self.send_command(0x12);
        self.wait_busy_high().await;
        self.send_command(0x02);
        self.wait_busy_low().await;
        Timer::after(Duration::from_millis(200)).await;
    }
    */

    /// Enter sleep mode
    pub async fn sleep(&mut self) {
        Timer::after(Duration::from_millis(100)).await;
        self.send_command(0x07);
        self.send_data(0xA5);
        Timer::after(Duration::from_millis(100)).await;
        self.pins.rst.set_low(); // Reset
    }
}

