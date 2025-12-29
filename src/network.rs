//! WiFi and HTTP networking for Pico W
//! Using reqwless for proper HTTP handling (chunked encoding, etc.)

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));

use defmt::*;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::Stack;
use reqwless::client::HttpClient;
use reqwless::request::Method;

/// Image buffer size: 600x448 pixels, 4 bits per pixel = 134_400 bytes
pub const IMAGE_BUFFER_SIZE: usize = 134_400;

/// Download raw 4bpp image from HTTP server using reqwless
/// Buffer must be provided by caller (allocated in heap in main)
pub async fn download_image<'a>(
    stack: &Stack<'_>,
    image_buffer: &'a mut [u8],
) -> Result<&'a [u8], &'static str> {
    if image_buffer.len() < IMAGE_BUFFER_SIZE {
        return Err("Buffer too small");
    }

    info!("Downloading image from: {}", IMAGE_URL);

    // Create HTTP client with reqwless
    let client_state = TcpClientState::<1, 4096, 4096>::new();
    let tcp_client = TcpClient::new(*stack, &client_state);
    let dns_client = DnsSocket::new(*stack);
    let mut http_client = HttpClient::new(&tcp_client, &dns_client);

    // Make HTTP GET request
    let mut request = http_client
        .request(Method::GET, IMAGE_URL)
        .await
        .map_err(|_| "Failed to create HTTP request")?;

    // Send request and get response
    let response = request
        .send(image_buffer)
        .await
        .map_err(|_| "Failed to send HTTP request")?;

    info!("Response status: {}", response.status.0);

    if response.status.0 != 200 {
        error!("HTTP error: status {}", response.status.0);
        return Err("HTTP request failed");
    }

    // Read response body
    let body_bytes = response
        .body()
        .read_to_end()
        .await
        .map_err(|_| "Failed to read response body")?;

    let body_len = body_bytes.len();
    info!("Downloaded {} bytes", body_len);

    // Log first 32 bytes for debugging
    if body_len >= 32 {
        debug!(
            "First 32 bytes: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
            body_bytes[0], body_bytes[1], body_bytes[2], body_bytes[3],
            body_bytes[4], body_bytes[5], body_bytes[6], body_bytes[7],
            body_bytes[8], body_bytes[9], body_bytes[10], body_bytes[11],
            body_bytes[12], body_bytes[13], body_bytes[14], body_bytes[15],
            body_bytes[16], body_bytes[17], body_bytes[18], body_bytes[19],
            body_bytes[20], body_bytes[21], body_bytes[22], body_bytes[23],
            body_bytes[24], body_bytes[25], body_bytes[26], body_bytes[27],
            body_bytes[28], body_bytes[29], body_bytes[30], body_bytes[31]
        );
    }

    if body_len != IMAGE_BUFFER_SIZE {
        warn!(
            "Image size mismatch: got {} bytes, expected {}",
            body_len, IMAGE_BUFFER_SIZE
        );
    }

    Ok(body_bytes)
}