use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use rppal::spi::{Bus, Mode, SlaveSelect, Spi};

use crate::config::{NeopixelEffect, NeopixelEffectType, StateEffects};
use crate::state::FeedbackState;

/// Animation step interval — 50Hz update rate.
const STEP_MS: u64 = 20;

/// SPI clock rate for WS2812 encoding. At ~6.4 MHz, each SPI bit ≈ 156ns.
/// Each WS2812 bit is encoded as 4 SPI bits → ~625ns per WS2812 bit,
/// within the protocol's ~1.25μs per-bit timing window.
const SPI_CLOCK_HZ: u32 = 6_400_000;

/// WS2812 reset frame: 50μs of low signal → ~40 zero bytes at 6.4 MHz.
const RESET_BYTES: usize = 40;

// ============================================================================
// Color utilities
// ============================================================================

/// Extract RGB channels from a 0xRRGGBB u32.
fn color_to_rgb(color: u32) -> (u8, u8, u8) {
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    (r, g, b)
}

/// Apply brightness scaling (0.0–1.0) to an RGB color.
fn apply_brightness(r: u8, g: u8, b: u8, brightness: f64) -> (u8, u8, u8) {
    let b_clamp = brightness.clamp(0.0, 1.0);
    (
        (r as f64 * b_clamp) as u8,
        (g as f64 * b_clamp) as u8,
        (b as f64 * b_clamp) as u8,
    )
}

/// Convert HSV (h: 0–360, s: 0.0–1.0, v: 0.0–1.0) to RGB.
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

// ============================================================================
// WS2812 SPI encoding
// ============================================================================

/// Encode a single WS2812 byte (8 bits) as SPI data (4 bytes).
///
/// Each WS2812 bit → 4 SPI bits:
///   "1" → 0b1100  (high for ~312ns, low for ~312ns)
///   "0" → 0b1000  (high for ~156ns, low for ~468ns)
fn encode_byte(byte: u8) -> [u8; 4] {
    let mut spi_bits: u32 = 0;
    for i in 0..8 {
        let ws_bit = (byte >> (7 - i)) & 1;
        let pattern: u32 = if ws_bit == 1 { 0b1100 } else { 0b1000 };
        spi_bits |= pattern << (28 - i * 4);
    }
    spi_bits.to_be_bytes()
}

/// Build the full SPI buffer for a strip of LEDs.
///
/// WS2812 expects GRB byte order per LED.
/// Buffer = LED data + reset frame (zeros).
fn build_spi_buffer(colors: &[(u8, u8, u8)]) -> Vec<u8> {
    let data_len = colors.len() * 12; // 3 bytes per LED × 4 SPI bytes per byte
    let mut buf = Vec::with_capacity(data_len + RESET_BYTES);

    for &(r, g, b) in colors {
        // WS2812 expects GRB order
        buf.extend_from_slice(&encode_byte(g));
        buf.extend_from_slice(&encode_byte(r));
        buf.extend_from_slice(&encode_byte(b));
    }

    // Reset frame
    buf.extend(std::iter::repeat(0u8).take(RESET_BYTES));
    buf
}

// ============================================================================
// Worker
// ============================================================================

/// Neopixel/WS2812 feedback provider worker.
///
/// Drives an addressable RGB LED strip via SPI. Each LED is individually
/// controllable with 24-bit color (8 bits each for R, G, B).
///
/// If `spi_device` is None, defaults to SPI bus 0, slave select 0.
pub fn worker(
    rx: mpsc::Receiver<FeedbackState>,
    _pin: u32, // GPIO pin — informational (SPI uses its own bus pins)
    led_count: u32,
    spi_device: Option<&str>,
    states: &StateEffects<NeopixelEffect>,
) {
    let mut spi = match create_spi(spi_device) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Neopixel feedback: failed to initialize SPI: {}", e);
            return;
        }
    };

    let num_leds = led_count as usize;
    let mut current_state = FeedbackState::Idle;

    // Turn off all LEDs initially
    send_colors(&mut spi, &vec![(0, 0, 0); num_leds]);

    loop {
        if let Some(effect) = states.get(current_state) {
            if !run_effect(&rx, &mut spi, num_leds, effect, &mut current_state) {
                break;
            }
        } else {
            // No config for this state — LEDs off, wait for next state
            send_colors(&mut spi, &vec![(0, 0, 0); num_leds]);
            match rx.recv() {
                Ok(s) => current_state = s,
                Err(_) => break,
            }
        }
    }

    // Turn off LEDs on shutdown
    send_colors(&mut spi, &vec![(0, 0, 0); num_leds]);
    log::debug!("Neopixel feedback worker shutting down ({} LEDs)", num_leds);
}

fn create_spi(device: Option<&str>) -> Result<Spi, rppal::spi::Error> {
    if let Some(_path) = device {
        // Custom SPI device path — use bus 0 SS 0 as default
        // rppal doesn't support arbitrary device paths directly,
        // so we log the intent and use the standard bus.
        log::warn!("Custom SPI device path not yet supported, using SPI0 CE0");
    }
    Spi::new(Bus::Spi0, SlaveSelect::Ss0, SPI_CLOCK_HZ, Mode::Mode0)
}

fn send_colors(spi: &mut Spi, colors: &[(u8, u8, u8)]) {
    let buf = build_spi_buffer(colors);
    if let Err(e) = spi.write(&buf) {
        log::warn!("Neopixel SPI write error: {}", e);
    }
}

/// Run an effect until a new state arrives. Returns false if channel is closed.
fn run_effect(
    rx: &mpsc::Receiver<FeedbackState>,
    spi: &mut Spi,
    num_leds: usize,
    effect: &NeopixelEffect,
    current_state: &mut FeedbackState,
) -> bool {
    let brightness = effect.brightness.unwrap_or(1.0);
    let base_color = effect.color.unwrap_or(0xFFFFFF);
    let (r, g, b) = color_to_rgb(base_color);
    let (r, g, b) = apply_brightness(r, g, b, brightness);

    match effect.effect {
        NeopixelEffectType::Off => {
            send_colors(spi, &vec![(0, 0, 0); num_leds]);
            match rx.recv() {
                Ok(s) => {
                    *current_state = s;
                    true
                }
                Err(_) => false,
            }
        }

        NeopixelEffectType::Solid => {
            send_colors(spi, &vec![(r, g, b); num_leds]);
            match rx.recv() {
                Ok(s) => {
                    *current_state = s;
                    true
                }
                Err(_) => false,
            }
        }

        NeopixelEffectType::Breathe => {
            let period_ms = effect.period_ms.unwrap_or(2000);
            let steps = period_ms / STEP_MS;
            let mut step: u64 = 0;

            loop {
                let phase = (step as f64 / steps as f64) * std::f64::consts::TAU;
                let scale = 0.5 - 0.5 * phase.cos();
                let sr = (r as f64 * scale) as u8;
                let sg = (g as f64 * scale) as u8;
                let sb = (b as f64 * scale) as u8;
                send_colors(spi, &vec![(sr, sg, sb); num_leds]);

                std::thread::sleep(Duration::from_millis(STEP_MS));
                step = (step + 1) % steps;

                match rx.try_recv() {
                    Ok(s) => {
                        *current_state = s;
                        return true;
                    }
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => return false,
                }
            }
        }

        NeopixelEffectType::Rainbow => {
            let period_ms = effect.period_ms.unwrap_or(2000);
            let steps = period_ms / STEP_MS;
            let mut step: u64 = 0;

            loop {
                let mut colors = Vec::with_capacity(num_leds);
                for i in 0..num_leds {
                    // Distribute hue across the strip, rotating over time
                    let hue = ((step as f64 / steps as f64) * 360.0
                        + (i as f64 / num_leds as f64) * 360.0)
                        % 360.0;
                    let (cr, cg, cb) = hsv_to_rgb(hue, 1.0, brightness);
                    colors.push((cr, cg, cb));
                }
                send_colors(spi, &colors);

                std::thread::sleep(Duration::from_millis(STEP_MS));
                step = (step + 1) % steps;

                match rx.try_recv() {
                    Ok(s) => {
                        *current_state = s;
                        return true;
                    }
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => return false,
                }
            }
        }

        NeopixelEffectType::Blink => {
            let period_ms = effect.period_ms.unwrap_or(500);
            let half_steps = (period_ms / 2) / STEP_MS;

            loop {
                // On phase
                send_colors(spi, &vec![(r, g, b); num_leds]);
                for _ in 0..half_steps {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }

                // Off phase
                send_colors(spi, &vec![(0, 0, 0); num_leds]);
                for _ in 0..half_steps {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_to_rgb_extracts_channels() {
        assert_eq!(color_to_rgb(0xFF0000), (255, 0, 0));
        assert_eq!(color_to_rgb(0x00FF00), (0, 255, 0));
        assert_eq!(color_to_rgb(0x0000FF), (0, 0, 255));
        assert_eq!(color_to_rgb(0x123456), (0x12, 0x34, 0x56));
        assert_eq!(color_to_rgb(0x000000), (0, 0, 0));
        assert_eq!(color_to_rgb(0xFFFFFF), (255, 255, 255));
    }

    #[test]
    fn apply_brightness_scales_correctly() {
        assert_eq!(apply_brightness(200, 100, 50, 0.5), (100, 50, 25));
        assert_eq!(apply_brightness(255, 255, 255, 0.0), (0, 0, 0));
        assert_eq!(apply_brightness(255, 255, 255, 1.0), (255, 255, 255));
    }

    #[test]
    fn apply_brightness_clamps() {
        assert_eq!(apply_brightness(100, 100, 100, 2.0), (100, 100, 100));
        assert_eq!(apply_brightness(100, 100, 100, -1.0), (0, 0, 0));
    }

    #[test]
    fn encode_byte_zero() {
        // 0b00000000 → all "0" patterns: 0b1000 per bit
        // 8 × 0b1000 packed into 32 bits:
        // 1000_1000_1000_1000_1000_1000_1000_1000 = 0x88888888
        assert_eq!(encode_byte(0x00), [0x88, 0x88, 0x88, 0x88]);
    }

    #[test]
    fn encode_byte_ff() {
        // 0xFF → all "1" patterns: 0b1100 per bit
        // 8 × 0b1100 packed into 32 bits:
        // 1100_1100_1100_1100_1100_1100_1100_1100 = 0xCCCCCCCC
        assert_eq!(encode_byte(0xFF), [0xCC, 0xCC, 0xCC, 0xCC]);
    }

    #[test]
    fn encode_byte_mixed() {
        // 0xA5 = 0b10100101
        // bit 7 (1): 1100
        // bit 6 (0): 1000
        // bit 5 (1): 1100
        // bit 4 (0): 1000
        // bit 3 (0): 1000
        // bit 2 (1): 1100
        // bit 1 (0): 1000
        // bit 0 (1): 1100
        // = 1100_1000_1100_1000_1000_1100_1000_1100 = 0xC8C88C8C
        assert_eq!(encode_byte(0xA5), [0xC8, 0xC8, 0x8C, 0x8C]);
    }

    #[test]
    fn build_spi_buffer_structure() {
        let colors = vec![(0xFF, 0x00, 0x80)]; // 1 LED: R=0xFF, G=0x00, B=0x80
        let buf = build_spi_buffer(&colors);

        // 1 LED × 3 bytes × 4 SPI bytes per byte = 12 data bytes + 40 reset bytes
        assert_eq!(buf.len(), 12 + RESET_BYTES);

        // GRB order: G first, then R, then B
        let g_encoded = encode_byte(0x00); // G=0x00
        let r_encoded = encode_byte(0xFF); // R=0xFF
        let b_encoded = encode_byte(0x80); // B=0x80

        assert_eq!(&buf[0..4], &g_encoded);
        assert_eq!(&buf[4..8], &r_encoded);
        assert_eq!(&buf[8..12], &b_encoded);

        // Reset frame: all zeros
        assert!(buf[12..].iter().all(|&b| b == 0));
    }

    #[test]
    fn build_spi_buffer_multiple_leds() {
        let colors = vec![(255, 0, 0), (0, 255, 0), (0, 0, 255)];
        let buf = build_spi_buffer(&colors);
        // 3 LEDs × 12 bytes + 40 reset
        assert_eq!(buf.len(), 36 + RESET_BYTES);
    }

    #[test]
    fn build_spi_buffer_empty() {
        let buf = build_spi_buffer(&[]);
        // Just reset bytes
        assert_eq!(buf.len(), RESET_BYTES);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn hsv_to_rgb_primary_colors() {
        // Red: H=0, S=1, V=1
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        // Green: H=120
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        // Blue: H=240
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
    }

    #[test]
    fn hsv_to_rgb_white_and_black() {
        // White: S=0, V=1
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), (255, 255, 255));
        // Black: V=0
        assert_eq!(hsv_to_rgb(0.0, 1.0, 0.0), (0, 0, 0));
    }
}
