//! WiFi and HTTP networking for Pico W
//! Using reqwless for proper HTTP handling (chunked encoding, etc.)

#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));

use defmt::*;
use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use reqwless::client::HttpClient;
use reqwless::request::Method;

/// Image buffer size: 600x448 pixels, 4 bits per pixel = 134_400 bytes
pub const IMAGE_BUFFER_SIZE: usize = 134_400;

/// Download raw 4bpp image from HTTP server using reqwless
/// Returns tuple: (image_data, next_delay_seconds)
/// Buffer must be provided by caller (allocated in heap in main)
pub async fn download_image<'a>(
    stack: &Stack<'_>,
    image_buffer: &'a mut [u8],
) -> Result<(&'a mut [u8], Option<u64>), &'static str> {
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

    // Parse X-Next-Delay header
    let mut next_delay: Option<u64> = None;
    for (name, value) in response.headers() {
        if name.eq_ignore_ascii_case("x-next-delay") {
            if let Ok(value_str) = core::str::from_utf8(value) {
                if let Ok(delay) = value_str.parse::<u64>() {
                    next_delay = Some(delay);
                    info!("X-Next-Delay header found: {} seconds", delay);
                } else {
                    warn!("Failed to parse X-Next-Delay value: {}", value_str);
                }
            }
            break;
        }
    }

    if next_delay.is_none() {
        info!("X-Next-Delay header not found, will use default interval");
    }

    // Read response body
    let body_bytes = response
        .body()
        .read_to_end()
        .await
        .map_err(|_| "Failed to read response body")?;

    let body_len = body_bytes.len();
    info!("Downloaded {} bytes", body_len);

    if body_len != IMAGE_BUFFER_SIZE {
        warn!(
            "Image size mismatch: got {} bytes, expected {}",
            body_len, IMAGE_BUFFER_SIZE
        );
    }

    Ok((body_bytes, next_delay))
}
