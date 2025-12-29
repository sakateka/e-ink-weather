//! WiFi and HTTP networking for Pico W
//! Adapted for embassy-rp 0.9.x, embassy-net 0.7.x, cyw43 0.6.x

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));

use defmt::*;
use embassy_net::tcp::TcpSocket;
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

/// Image buffer size: 600x448 pixels, 4 bits per pixel = 134_400 bytes
pub const IMAGE_BUFFER_SIZE: usize = 134_400;

/// Download raw 4bpp image from HTTP server
pub async fn download_image(stack: &Stack<'_>) -> Result<&'static [u8], &'static str> {
    info!(
        "Downloading image from: {}:{}{}",
        SERVER_IP, SERVER_PORT, API_PATH
    );

    // Parse IP address
    let remote_addr = parse_ip(SERVER_IP)?;

    info!("Connecting to server");

    // Create TCP socket
    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 1024];
    let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(30)));

    // Connect to server
    socket
        .connect((remote_addr, SERVER_PORT))
        .await
        .map_err(|_| "TCP connect failed")?;

    info!("Connected to server");

    // Send HTTP GET request
    let mut request_buf = [0u8; 512];
    let request_len = format_http_request(&mut request_buf, SERVER_IP, API_PATH)?;

    // Write all data
    let mut written = 0;
    while written < request_len {
        let n = socket
            .write(&request_buf[written..request_len])
            .await
            .map_err(|_| "Failed to send HTTP request")?;
        written += n;
    }

    info!("HTTP request sent, reading response...");

    // Use static buffer to avoid stack overflow
    static IMAGE_BUFFER: StaticCell<[u8; IMAGE_BUFFER_SIZE]> = StaticCell::new();
    let image_buffer = IMAGE_BUFFER.init([0u8; IMAGE_BUFFER_SIZE]);
    let mut response_len = 0;
    let mut header_complete = false;
    let mut temp_buf = [0u8; 512];
    let mut header_buf = [0u8; 1024]; // Temporary buffer for headers
    let mut header_len = 0;

    loop {
        let n = socket
            .read(&mut temp_buf)
            .await
            .map_err(|_| "Socket read failed")?;

        if n == 0 {
            break; // Connection closed
        }

        if !header_complete {
            // Accumulate data in header buffer
            if header_len + n > header_buf.len() {
                return Err("Headers too large");
            }
            header_buf[header_len..header_len + n].copy_from_slice(&temp_buf[..n]);
            header_len += n;

            // Check if we have complete headers
            if let Some(header_end) = find_header_end(&header_buf[..header_len]) {
                header_complete = true;

                // Copy body data to image buffer
                let body_start = header_end + 4; // Skip \r\n\r\n
                let body_in_header = header_len - body_start;
                if body_in_header > 0 {
                    image_buffer[..body_in_header]
                        .copy_from_slice(&header_buf[body_start..header_len]);
                    response_len = body_in_header;
                }

                info!("Headers parsed, body so far: {} bytes", response_len);
            }
        } else {
            // Already past headers, accumulate body directly into image buffer
            if response_len + n > IMAGE_BUFFER_SIZE {
                return Err("Response too large");
            }
            image_buffer[response_len..response_len + n].copy_from_slice(&temp_buf[..n]);
            response_len += n;
        }

        // Check if we have enough data
        if response_len >= IMAGE_BUFFER_SIZE {
            info!("Received complete response: {} bytes", response_len);
            break;
        }
    }

    socket.close();

    if response_len != IMAGE_BUFFER_SIZE {
        warn!(
            "Image size mismatch: got {} bytes, expected {}",
            response_len, IMAGE_BUFFER_SIZE
        );
    }

    info!("Download complete: {} bytes", response_len);
    Ok(&image_buffer[..response_len])
}

/// Parse IP address string into Ipv4Address
fn parse_ip(ip_str: &str) -> Result<embassy_net::Ipv4Address, &'static str> {
    let parts: heapless::Vec<&str, 4> = ip_str.split('.').collect();
    if parts.len() != 4 {
        return Err("Invalid IP address format");
    }

    let mut octets = [0u8; 4];
    for (i, part) in parts.iter().enumerate() {
        octets[i] = part.parse().map_err(|_| "Invalid IP octet")?;
    }

    Ok(embassy_net::Ipv4Address::new(
        octets[0], octets[1], octets[2], octets[3],
    ))
}

/// Format HTTP GET request into buffer
fn format_http_request(buf: &mut [u8], host: &str, path: &str) -> Result<usize, &'static str> {
    use core::fmt::Write as _;
    let mut cursor = heapless::String::<512>::new();

    core::write!(&mut cursor, "GET {} HTTP/1.1\r\n", path).map_err(|_| "Request too long")?;
    core::write!(&mut cursor, "Host: {}\r\n", host).map_err(|_| "Request too long")?;
    core::write!(&mut cursor, "Connection: close\r\n").map_err(|_| "Request too long")?;
    core::write!(&mut cursor, "\r\n").map_err(|_| "Request too long")?;

    let bytes = cursor.as_bytes();
    if bytes.len() > buf.len() {
        return Err("Request buffer too small");
    }

    buf[..bytes.len()].copy_from_slice(bytes);
    Ok(bytes.len())
}

/// Find end of HTTP headers (\r\n\r\n)
fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

/// Wait for specified number of minutes
pub async fn wait_minutes(minutes: u32) {
    let duration = Duration::from_secs(minutes as u64 * 60);
    info!("Sleeping for {} minutes", minutes);
    Timer::after(duration).await;
}
