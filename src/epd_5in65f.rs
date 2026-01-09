//! Driver for 5.65 inch e-Paper display (600x448 pixels)
//! Bit-banged SPI over GPIO, aligned with Waveshare C reference.

use embassy_time::{Duration, Timer};

use crate::config::EpdPins;

/// Display dimensions
pub const EPD_5IN65F_WIDTH: u16 = 600;
pub const EPD_5IN65F_HEIGHT: u16 = 448;

/// Colors: 3-bit indices matching lib/epd_5in65f.h
pub const EPD_5IN65F_BLACK: u8 = 0x0;
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
        defmt::debug!(
            "wait_busy_high: starting, current state={}",
            self.pins.busy.is_high()
        );
        let mut iterations = 0u32;
        while !self.pins.busy.is_high() {
            Timer::after(Duration::from_millis(1)).await;
            iterations += 1;
            if iterations & 127 == 0 {
                defmt::debug!(
                    "wait_busy_high: still waiting, iterations={}, state={}",
                    iterations,
                    self.pins.busy.is_high()
                );
            }
        }
        defmt::debug!(
            "wait_busy_high: done after {} iterations, final state={}",
            iterations,
            self.pins.busy.is_high()
        );
    }

    /// Wait until BUSY becomes low
    async fn wait_busy_low(&mut self) {
        defmt::debug!(
            "wait_busy_low: starting, current state={}",
            self.pins.busy.is_high()
        );
        let mut iterations = 0u32;
        while self.pins.busy.is_high() {
            Timer::after(Duration::from_millis(1)).await;
            iterations += 1;
            if iterations & 127 == 0 {
                defmt::debug!(
                    "wait_busy_low: still waiting, iterations={}, state={}",
                    iterations,
                    self.pins.busy.is_high()
                );
            }
        }
        defmt::debug!(
            "wait_busy_low: done after {} iterations, final state={}",
            iterations,
            self.pins.busy.is_high()
        );
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

/// Simple 5x7 bitmap font for digits 0-9
/// Each digit is 5 bytes (5 columns), each byte represents 7 pixels (bits 0-6)
const FONT_5X7: [[u8; 5]; 10] = [
    [0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b0111110], // 0
    [0b0000000, 0b0100001, 0b1111111, 0b0000001, 0b0000000], // 1
    [0b0100011, 0b1000101, 0b1001001, 0b1010001, 0b0100001], // 2
    [0b0100010, 0b1000001, 0b1001001, 0b1001001, 0b0110110], // 3
    [0b0001100, 0b0010100, 0b0100100, 0b1111111, 0b0000100], // 4
    [0b1110010, 0b1010001, 0b1010001, 0b1010001, 0b1001110], // 5
    [0b0111110, 0b1001001, 0b1001001, 0b1001001, 0b0000110], // 6
    [0b1000000, 0b1000111, 0b1001000, 0b1010000, 0b1100000], // 7
    [0b0110110, 0b1001001, 0b1001001, 0b1001001, 0b0110110], // 8
    [0b0110000, 0b1001001, 0b1001001, 0b1001001, 0b0111110], // 9
];

/// Draw a single digit at position (x, y) in the image buffer
/// Scale factor determines the size (1 = 5x7, 2 = 10x14, etc.)
fn draw_digit(image: &mut [u8], x: u16, y: u16, digit: u8, color: u8, scale: u16) {
    if digit > 9 {
        return;
    }

    let glyph = &FONT_5X7[digit as usize];
    let width_half = EPD_5IN65F_WIDTH / 2;

    for col in 0..5 {
        let column_data = glyph[col];
        for row in 0..7 {
            if (column_data & (1 << (6 - row))) != 0 {
                // Draw scaled pixel
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = x + (col as u16 * scale) + sx;
                        let py = y + (row as u16 * scale) + sy;

                        if px < EPD_5IN65F_WIDTH && py < EPD_5IN65F_HEIGHT {
                            set_pixel(image, px, py, color, width_half);
                        }
                    }
                }
            }
        }
    }
}

/// Set a single pixel in the image buffer
/// Image format: 4bpp packed (two pixels per byte), row-major
fn set_pixel(image: &mut [u8], x: u16, y: u16, color: u8, width_half: u16) {
    let byte_index = (x / 2 + width_half * y) as usize;

    if byte_index < image.len() {
        if x.is_multiple_of(2) {
            // High nibble (left pixel)
            image[byte_index] = (image[byte_index] & 0x0F) | ((color & 0x0F) << 4);
        } else {
            // Low nibble (right pixel)
            image[byte_index] = (image[byte_index] & 0xF0) | (color & 0x0F);
        }
    }
}

/// Draw a number (up to 3 digits) at position (x, y)
/// Returns the width of the drawn text in pixels
pub fn draw_number(image: &mut [u8], x: u16, y: u16, number: u8, color: u8, scale: u16) -> u16 {
    let mut current_x = x;
    let char_width = 5 * scale;
    let char_spacing = 2 * scale;

    if number >= 100 {
        let hundreds = number / 100;
        draw_digit(image, current_x, y, hundreds, color, scale);
        current_x += char_width + char_spacing;
    }

    if number >= 10 {
        let tens = (number / 10) % 10;
        draw_digit(image, current_x, y, tens, color, scale);
        current_x += char_width + char_spacing;
    }

    let ones = number % 10;
    draw_digit(image, current_x, y, ones, color, scale);
    current_x += char_width;

    current_x - x
}
