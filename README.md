# Weather e-Paper Display

Embedded Rust project for Raspberry Pi Pico W with Waveshare 5.65" e-Paper display.

## Hardware

- **Microcontroller**: [Raspberry Pi Pico W](https://www.raspberrypi.com/products/raspberry-pi-pico/)
- **Display**: [Waveshare Pico-ePaper-5.65](https://www.waveshare.com/wiki/Pico-ePaper-5.65) (600×448, 7-color)

## Features

- Downloads weather image via HTTP
- Displays on 5.65" e-Paper (4bpp, 7-color)
- WiFi connectivity
- Button controls:
  - **KEY0**: Refresh display immediately
  - **KEY1**: Blink onboard LED
- Automatic updates every N minutes (configurable)

## Quick Start

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add ARM Cortex-M0+ target
rustup target add thumbv6m-none-eabi

# Install probe-rs for flashing
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh

# Install flip-link (stack overflow protection)
cargo install flip-link
```

### Configuration

1. Copy `default.toml` to `local.toml`:
```bash
cp default.toml local.toml
```

2. Edit `local.toml`:
```toml
[wifi]
ssid = "YourWiFiSSID"
password = "YourWiFiPassword"

[image]
url = "http://your-server.com/weather-image.raw"
update_interval_minutes = 30
```

### Build & Flash

#### Method 1: USB Bootloader (UF2)

1. Generate UF2 file:
```bash
# Install elf2uf2-rs
cargo install elf2uf2-rs

# Build and convert to UF2
cargo build --release
elf2uf2-rs target/thumbv6m-none-eabi/release/pico-epaper pico-epaper.uf2
```

2. Flash via bootloader:
   - Hold **BOOTSEL** button on Pico W
   - Connect USB cable (while holding BOOTSEL)
   - Pico W appears as USB mass storage device
   - Copy `pico-epaper.uf2` to the drive
   - Device automatically reboots and runs firmware

**Learn more**: [Getting Started with Pico](https://datasheets.raspberrypi.com/pico/getting-started-with-pico.pdf) (Chapter 3)

#### Method 2: SWD Debug Probe (Recommended for Development)

Requires a debug probe (e.g., another Pico, Raspberry Pi Debug Probe, or any SWD-compatible debugger).

```bash
# Flash via SWD
cargo run --release

# Or manually with probe-rs
probe-rs run --chip RP2040 --protocol swd target/thumbv6m-none-eabi/release/pico-epaper
```

**Setup guides**:
- [Connecting Debug Probe to Pico](https://www.raspberrypi.com/documentation/microcontrollers/debug-probe.html)
- [What is SWD?](https://developer.arm.com/documentation/ihi0031/a/The-Serial-Wire-Debug-Port--SW-DP-) - ARM Serial Wire Debug protocol
- [Using Pico as Debug Probe](https://github.com/raspberrypi/debugprobe)

**SWD Connections**:
| Probe Pin | Pico W Pin | Description |
|-----------|------------|-------------|
| SWCLK     | SWCLK      | Clock signal |
| SWDIO     | SWDIO      | Data I/O |
| GND       | GND        | Ground |
| 3V3       | 3V3(OUT)   | Power (optional) |

### View Logs

```bash
# Real-time logs via defmt (requires SWD probe)
cargo run --release
```

## Image Format

The display expects raw 4bpp image data:
- **Size**: 600×448 pixels
- **Format**: 4 bits per pixel (2 pixels per byte)
- **Total**: 134,400 bytes
- **Byte format**: `[pixel0_high_nibble | pixel1_low_nibble]`
- **Order**: Row-major (left-to-right, top-to-bottom)

### Color Palette

```
0: Black
1: White
2: Green
3: Blue
4: Red
5: Yellow
6: Orange
7: Clean
```

### Generate Image

Use this fork to generate `dashboard.raw` image blobs: [pi-inky-weather-epd](https://github.com/sakateka/pi-inky-weather-epd)

The fork generates properly formatted 4bpp raw images compatible with this display driver.

## Pin Mapping

| Function | GPIO | Description |
|----------|------|-------------|
| EPD_RST  | 12   | Display reset |
| EPD_DC   | 8    | Data/Command |
| EPD_CS   | 9    | Chip select |
| EPD_BUSY | 13   | Busy signal |
| EPD_CLK  | 10   | SPI clock |
| EPD_MOSI | 11   | SPI data |
| KEY0     | 15   | Button 0 (refresh) |
| KEY1     | 17   | Button 1 (LED blink) |
| KEY2     | 2    | Button 2 (unused) |

## Troubleshooting

### Display not updating
- Check BUSY pin connection (GPIO13)
- Verify SPI pins (CLK=10, MOSI=11)
- Check power supply (5V required for e-Paper)

### WiFi connection fails
- Verify SSID/password in `local.toml`
- Check WiFi signal strength
- Ensure 2.4GHz network (Pico W doesn't support 5GHz)

### Image corrupted
- Verify image size is exactly 134,400 bytes
- Check HTTP server returns raw binary (not chunked encoding)
- Use `reqwless` library (handles HTTP properly)

## License

MIT