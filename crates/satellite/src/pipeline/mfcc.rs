//! MFCC (Mel-Frequency Cepstral Coefficients) pipeline stage.
//!
//! Observation-only: computes spectral features from each audio frame without
//! modifying samples. Always returns `true` from `process()`.
//!
//! Uses 192-sample overlap from the previous frame to assemble a 512-sample
//! FFT window (192 prev + 320 current), giving genuine 31.25 Hz frequency
//! resolution at 16 kHz sample rate.

use std::sync::Arc;

use realfft::{num_complex::Complex, RealFftPlanner, RealToComplex};

use super::{Stage, StagePending, StatKey};

/// Sparse mel filterbank entry: triangular filter covering a range of FFT bins.
struct MelFilter {
    start_bin: usize,
    weights: Vec<f32>,
}

const RING_CAPACITY: usize = 100; // ~2s at 50Hz frame rate

// Auto-ranging: minimum dynamic range for quantization (prevents
// pure-silence frames from stretching noise to fill 0-255).
const MIN_DYNAMIC_RANGE: f32 = 5.0;
// EMA rate for floor/ceil adaptation.  Floor rises slowly (~5s at 50Hz),
// ceil decays at medium speed (~1s).  Both snap instantly when exceeded.
const FLOOR_RISE_ALPHA: f32 = 0.004;
const CEIL_DECAY_ALPHA: f32 = 0.02;

pub struct MfccStage {
    // Overlap buffer: last (n_fft - frame_size) samples from previous frame
    prev_tail: Vec<f32>,

    // Hann window (precomputed at init)
    hann_window: Vec<f32>,

    // FFT
    fft: Arc<dyn RealToComplex<f32>>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,

    // Power spectrum scratch
    power_spectrum: Vec<f32>,

    // Mel filterbank (sparse, precomputed)
    mel_filters: Vec<MelFilter>,
    mel_energies: Vec<f32>,

    // DCT-II matrix (precomputed, row-major: n_mfcc × n_mels)
    dct_matrix: Vec<f32>,
    n_mfcc: usize,
    n_mels: usize,
    n_fft: usize,
    frame_size: usize,
    overlap: usize,

    // Output coefficients (current frame)
    mfcc_coeffs: Vec<f32>,

    // Ring buffer for future wake word matcher
    ring: Vec<Vec<f32>>,
    ring_pos: usize,
    ring_len: usize,

    // Quantized log-mel energies for spectrogram (u8, 0-255)
    quantized_mel: Vec<u8>,
    // Auto-ranging: adapts to actual signal level for good contrast
    mel_range_floor: f32,
    mel_range_ceil: f32,

    // Summary stats for diagnostics
    stats_buf: [(StatKey, f64); 2],
    sample_rate: u32,
}

impl MfccStage {
    pub fn new(
        n_mfcc: usize,
        n_mels: usize,
        n_fft: usize,
        frame_size: usize,
        sample_rate: u32,
    ) -> Self {
        assert!(n_fft >= frame_size, "n_fft must be >= frame_size");

        let overlap = n_fft - frame_size;
        let spectrum_len = n_fft / 2 + 1;

        // Precompute Hann window
        let hann_window: Vec<f32> = (0..n_fft)
            .map(|n| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (n_fft - 1) as f32).cos())
            })
            .collect();

        // Setup FFT
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n_fft);
        let fft_input = fft.make_input_vec();
        let fft_output = fft.make_output_vec();
        let fft_scratch = fft.make_scratch_vec();

        // Build mel filterbank
        let mel_filters = build_mel_filterbank(n_mels, n_fft, sample_rate);

        // Precompute DCT-II matrix
        let dct_matrix = build_dct_matrix(n_mfcc, n_mels);

        // Pre-allocate ring buffer slots
        let ring: Vec<Vec<f32>> = (0..RING_CAPACITY)
            .map(|_| vec![0.0; n_mfcc])
            .collect();

        Self {
            prev_tail: vec![0.0; overlap],
            hann_window,
            fft,
            fft_input,
            fft_output,
            fft_scratch,
            power_spectrum: vec![0.0; spectrum_len],
            mel_filters,
            mel_energies: vec![0.0; n_mels],
            dct_matrix,
            n_mfcc,
            n_mels,
            n_fft,
            frame_size,
            overlap,
            mfcc_coeffs: vec![0.0; n_mfcc],
            quantized_mel: vec![0u8; n_mels],
            mel_range_floor: 0.0,
            mel_range_ceil: 25.0,
            ring,
            ring_pos: 0,
            ring_len: 0,
            stats_buf: [
                (StatKey::SpectralCentroid, 0.0),
                (StatKey::SpectralFlatness, 0.0),
            ],
            sample_rate,
        }
    }
}

impl MfccStage {
    /// Quantized log-mel energies (u8, 0-255) for spectrogram display.
    pub fn mel_bytes(&self) -> &[u8] {
        &self.quantized_mel
    }
}

impl Stage for MfccStage {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        let n = samples.len().min(self.frame_size);

        // 1. Assemble 512-sample window: overlap (192) + current frame (320)
        //    Copy prev_tail into fft_input[0..overlap]
        self.fft_input[..self.overlap].copy_from_slice(&self.prev_tail);

        //    Convert i16 samples → f32 into fft_input[overlap..overlap+n]
        for (i, &s) in samples[..n].iter().enumerate() {
            self.fft_input[self.overlap + i] = s as f32;
        }

        //    Zero-pad any remainder (shouldn't happen at 320 samples)
        for v in &mut self.fft_input[self.overlap + n..] {
            *v = 0.0;
        }

        // 2. Apply Hann window
        for (x, &w) in self.fft_input.iter_mut().zip(self.hann_window.iter()) {
            *x *= w;
        }

        // 3. Save tail for next frame (from the UN-windowed current frame samples)
        //    We need the raw samples, not the windowed ones. Save from current frame.
        //    The overlap region comes from the end of the current frame:
        //    fft_input[frame_size..n_fft] before windowing = samples[frame_size - overlap..frame_size]
        //    But we already overwrote fft_input. Save from samples directly.
        if n >= self.overlap {
            for (i, &s) in samples[n - self.overlap..n].iter().enumerate() {
                self.prev_tail[i] = s as f32;
            }
        } else {
            // Short frame: shift prev_tail left, fill end with new samples
            let keep = self.overlap - n;
            let tail_len = self.prev_tail.len();
            self.prev_tail.copy_within(tail_len - keep.., 0);
            for (i, &s) in samples[..n].iter().enumerate() {
                self.prev_tail[keep + i] = s as f32;
            }
        }

        // 4. FFT → complex spectrum
        self.fft
            .process_with_scratch(&mut self.fft_input, &mut self.fft_output, &mut self.fft_scratch)
            .ok();

        // 5. Power spectrum: |bin|² = re² + im²
        for (ps, c) in self.power_spectrum.iter_mut().zip(self.fft_output.iter()) {
            *ps = c.re * c.re + c.im * c.im;
        }

        // 6. Spectral centroid and flatness from power spectrum
        let (centroid, flatness) = compute_spectral_stats(
            &self.power_spectrum,
            self.sample_rate,
            self.n_fft,
        );
        self.stats_buf[0].1 = centroid as f64;
        self.stats_buf[1].1 = flatness as f64;

        // 7. Apply mel filterbank → log mel energies
        for (i, filter) in self.mel_filters.iter().enumerate() {
            let mut sum = 0.0f32;
            for (j, &w) in filter.weights.iter().enumerate() {
                let bin = filter.start_bin + j;
                if bin < self.power_spectrum.len() {
                    sum += self.power_spectrum[bin] * w;
                }
            }
            self.mel_energies[i] = (sum.max(1e-10)).ln();
        }

        // 8. Auto-range and quantize log-mel energies to u8 for spectrogram
        {
            let frame_min = self.mel_energies.iter().cloned().fold(f32::MAX, f32::min);
            let frame_max = self.mel_energies.iter().cloned().fold(f32::MIN, f32::max);

            // Floor: instant snap down, slow rise (tracks ambient noise)
            if frame_min < self.mel_range_floor {
                self.mel_range_floor = frame_min;
            } else {
                self.mel_range_floor += FLOOR_RISE_ALPHA * (frame_min - self.mel_range_floor);
            }
            // Ceil: instant snap up, medium decay (tracks speech peaks)
            if frame_max > self.mel_range_ceil {
                self.mel_range_ceil = frame_max;
            } else {
                self.mel_range_ceil += CEIL_DECAY_ALPHA * (frame_max - self.mel_range_ceil);
            }

            let range = (self.mel_range_ceil - self.mel_range_floor).max(MIN_DYNAMIC_RANGE);
            let floor = self.mel_range_floor;
            for (q, &e) in self.quantized_mel.iter_mut().zip(self.mel_energies.iter()) {
                *q = ((e - floor) / range * 255.0).clamp(0.0, 255.0) as u8;
            }
        }

        // 9. DCT-II: mfcc_coeffs[k] = sum(dct_matrix[k][n] * log_mel[n])
        for k in 0..self.n_mfcc {
            let row_start = k * self.n_mels;
            let mut sum = 0.0f32;
            for n in 0..self.n_mels {
                sum += self.dct_matrix[row_start + n] * self.mel_energies[n];
            }
            self.mfcc_coeffs[k] = sum;
        }

        // 10. Push to ring buffer
        self.ring[self.ring_pos].copy_from_slice(&self.mfcc_coeffs);
        self.ring_pos = (self.ring_pos + 1) % RING_CAPACITY;
        if self.ring_len < RING_CAPACITY {
            self.ring_len += 1;
        }

        true // observation-only
    }

    fn analyze(self, _pending: &mut StagePending) -> Self
    where
        Self: Sized,
    {
        self // no runtime config changes
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "mfcc"
    }

    fn reset(&mut self) {
        self.prev_tail.fill(0.0);
        self.mel_range_floor = 0.0;
        self.mel_range_ceil = 25.0;
        self.ring_pos = 0;
        self.ring_len = 0;
        for slot in &mut self.ring {
            slot.fill(0.0);
        }
    }
}

// ── Mel filterbank construction ─────────────────────────────────────────

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
}

fn build_mel_filterbank(n_mels: usize, n_fft: usize, sample_rate: u32) -> Vec<MelFilter> {
    let nyquist = sample_rate as f32 / 2.0;
    let spectrum_len = n_fft / 2 + 1;
    let bin_hz = sample_rate as f32 / n_fft as f32;

    let mel_low = hz_to_mel(0.0);
    let mel_high = hz_to_mel(nyquist);

    // n_mels + 2 evenly-spaced points on mel scale
    let n_points = n_mels + 2;
    let mel_points: Vec<f32> = (0..n_points)
        .map(|i| mel_low + (mel_high - mel_low) * i as f32 / (n_points - 1) as f32)
        .collect();

    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bin_points: Vec<f32> = hz_points.iter().map(|&h| h / bin_hz).collect();

    let mut filters = Vec::with_capacity(n_mels);
    for i in 0..n_mels {
        let left = bin_points[i];
        let center = bin_points[i + 1];
        let right = bin_points[i + 2];

        let start_bin = left.floor() as usize;
        let end_bin = (right.ceil() as usize).min(spectrum_len - 1);

        if start_bin >= end_bin {
            filters.push(MelFilter {
                start_bin,
                weights: vec![0.0],
            });
            continue;
        }

        let mut weights = Vec::with_capacity(end_bin - start_bin + 1);
        for bin in start_bin..=end_bin {
            let freq = bin as f32;
            let w = if freq <= left {
                0.0
            } else if freq <= center {
                (freq - left) / (center - left)
            } else if freq <= right {
                (right - freq) / (right - center)
            } else {
                0.0
            };
            weights.push(w);
        }

        filters.push(MelFilter { start_bin, weights });
    }

    filters
}

// ── DCT-II matrix ───────────────────────────────────────────────────────

fn build_dct_matrix(n_mfcc: usize, n_mels: usize) -> Vec<f32> {
    let mut matrix = vec![0.0f32; n_mfcc * n_mels];
    let scale = (2.0 / n_mels as f32).sqrt();
    for k in 0..n_mfcc {
        for n in 0..n_mels {
            matrix[k * n_mels + n] =
                scale * (std::f32::consts::PI * k as f32 * (n as f32 + 0.5) / n_mels as f32).cos();
        }
    }
    matrix
}

// ── Spectral statistics ─────────────────────────────────────────────────

fn compute_spectral_stats(power_spectrum: &[f32], sample_rate: u32, n_fft: usize) -> (f32, f32) {
    let bin_hz = sample_rate as f32 / n_fft as f32;
    let mut weighted_sum = 0.0f64;
    let mut total_power = 0.0f64;
    let mut log_sum = 0.0f64;
    let n = power_spectrum.len();

    for (i, &p) in power_spectrum.iter().enumerate() {
        let freq = i as f64 * bin_hz as f64;
        let p64 = p as f64;
        weighted_sum += freq * p64;
        total_power += p64;
        log_sum += (p64.max(1e-30)).ln();
    }

    // Spectral centroid (Hz)
    let centroid = if total_power > 0.0 {
        (weighted_sum / total_power) as f32
    } else {
        0.0
    };

    // Spectral flatness: geometric mean / arithmetic mean
    // geometric mean = exp(mean(log(x)))
    let flatness = if n > 0 && total_power > 0.0 {
        let geo_mean = (log_sum / n as f64).exp();
        let arith_mean = total_power / n as f64;
        (geo_mean / arith_mean).min(1.0) as f32
    } else {
        0.0
    };

    (centroid, flatness)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mfcc_stage_creates_with_defaults() {
        let stage = MfccStage::new(13, 26, 512, 320, 16000);
        assert_eq!(stage.n_mfcc, 13);
        assert_eq!(stage.n_mels, 26);
        assert_eq!(stage.n_fft, 512);
        assert_eq!(stage.frame_size, 320);
        assert_eq!(stage.overlap, 192);
        assert_eq!(stage.prev_tail.len(), 192);
        assert_eq!(stage.hann_window.len(), 512);
        assert_eq!(stage.mel_filters.len(), 26);
        assert_eq!(stage.dct_matrix.len(), 13 * 26);
    }

    #[test]
    fn mfcc_process_returns_true() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let mut samples = vec![0i16; 320];
        assert!(stage.process(&mut samples));
    }

    #[test]
    fn mfcc_process_silence() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let mut samples = vec![0i16; 320];
        stage.process(&mut samples);

        let stats = stage.stats();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].0, StatKey::SpectralCentroid);
        assert_eq!(stats[1].0, StatKey::SpectralFlatness);
        // Silence: centroid should be ~0, flatness ~0
        assert!(stats[0].1.abs() < 1.0, "centroid for silence should be near 0");
    }

    #[test]
    fn mfcc_process_tone() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        // Generate a 1kHz tone
        let mut samples: Vec<i16> = (0..320)
            .map(|i| {
                (10000.0 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 16000.0).sin()) as i16
            })
            .collect();

        // Process two frames to fill overlap
        stage.process(&mut samples.clone());
        stage.process(&mut samples);

        let stats = stage.stats();
        let centroid = stats[0].1;
        // With a 1kHz tone, centroid should be near 1000 Hz
        assert!(centroid > 500.0 && centroid < 2000.0,
            "centroid for 1kHz tone should be near 1000, got {}", centroid);

        // Flatness should be low (tonal, not noisy)
        let flatness = stats[1].1;
        assert!(flatness < 0.5, "flatness for pure tone should be low, got {}", flatness);
    }

    #[test]
    fn mfcc_ring_buffer_fills() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let mut samples = vec![1000i16; 320];

        for _ in 0..5 {
            stage.process(&mut samples);
        }
        assert_eq!(stage.ring_len, 5);
        assert_eq!(stage.ring_pos, 5);
    }

    #[test]
    fn mfcc_reset_clears_state() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let mut samples = vec![1000i16; 320];
        stage.process(&mut samples);
        assert_eq!(stage.ring_len, 1);

        stage.reset();
        assert_eq!(stage.ring_len, 0);
        assert_eq!(stage.ring_pos, 0);
        assert!(stage.prev_tail.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn mel_filterbank_correct_count() {
        let filters = build_mel_filterbank(26, 512, 16000);
        assert_eq!(filters.len(), 26);

        // First filter should start near bin 0
        assert!(filters[0].start_bin <= 2);

        // All filters should have non-empty weights
        for f in &filters {
            assert!(!f.weights.is_empty());
        }
    }

    #[test]
    fn dct_matrix_dimensions() {
        let matrix = build_dct_matrix(13, 26);
        assert_eq!(matrix.len(), 13 * 26);
    }

    #[test]
    fn hann_window_endpoints() {
        let n = 512;
        let w: Vec<f32> = (0..n)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos()))
            .collect();
        // Hann window: endpoints should be ~0, middle should be ~1
        assert!(w[0] < 0.01);
        assert!(w[n - 1] < 0.01);
        assert!((w[n / 2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn spectral_stats_white_noise() {
        // Flat power spectrum → flatness should be near 1.0
        let spectrum: Vec<f32> = vec![100.0; 257];
        let (centroid, flatness) = compute_spectral_stats(&spectrum, 16000, 512);

        // Centroid should be near Nyquist/2 for flat spectrum
        assert!(centroid > 2000.0 && centroid < 6000.0,
            "centroid for flat spectrum: {}", centroid);
        // Flatness should be near 1.0
        assert!(flatness > 0.95, "flatness for flat spectrum: {}", flatness);
    }

    #[test]
    fn mfcc_does_not_modify_samples() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let original: Vec<i16> = (0..320).map(|i| (i * 100) as i16).collect();
        let mut samples = original.clone();
        stage.process(&mut samples);
        assert_eq!(samples, original, "MFCC stage must not modify samples");
    }

    #[test]
    fn mfcc_stage_name() {
        let stage = MfccStage::new(13, 26, 512, 320, 16000);
        assert_eq!(stage.name(), "mfcc");
    }

    #[test]
    fn quantized_mel_has_correct_length() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        let mut samples = vec![0i16; 320];
        stage.process(&mut samples);

        let mel = stage.mel_bytes();
        assert_eq!(mel.len(), 26);
    }

    #[test]
    fn quantized_mel_auto_range_adapts() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);

        // Process several quiet frames to stabilize the range
        let mut silence = vec![0i16; 320];
        for _ in 0..10 {
            stage.process(&mut silence);
        }

        // Now a loud signal should push the ceiling up and produce high values
        let mut loud: Vec<i16> = (0..320).map(|i| ((i * 7919) % 65536) as i16).collect();
        stage.process(&mut loud.clone());
        stage.process(&mut loud);

        let mel = stage.mel_bytes();
        let max_val = mel.iter().copied().max().unwrap_or(0);
        assert!(max_val > 200, "loud signal should produce high mel values after auto-range, max was {}", max_val);
    }

    #[test]
    fn quantized_mel_uses_full_range_for_tonal_signal() {
        let mut stage = MfccStage::new(13, 26, 512, 320, 16000);
        // 1kHz tone: energy concentrated in a few mel bands
        let mut tone: Vec<i16> = (0..320)
            .map(|i| (10000.0 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 16000.0).sin()) as i16)
            .collect();

        // Process enough frames for auto-range to stabilize
        for _ in 0..20 {
            stage.process(&mut tone.clone());
        }
        stage.process(&mut tone);

        let mel = stage.mel_bytes();
        let min_val = mel.iter().copied().min().unwrap_or(255);
        let max_val = mel.iter().copied().max().unwrap_or(0);
        // Auto-range should spread the values — some bands near 0 (quiet), some near 255 (tone)
        assert!(max_val > 200, "peak band should be bright, got {}", max_val);
        assert!(min_val < 50, "quiet bands should be dark, got {}", min_val);
    }
}
