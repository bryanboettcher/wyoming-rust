use super::{Stage, StagePending, StatKey};

/// Second-order Butterworth biquad high-pass filter.
///
/// Removes sub-vocal rumble (HVAC, traffic, refrigerator hum) that
/// inflates VAD energy readings without carrying speech information.
pub struct HighPassFilter {
    b0: f32, b1: f32, b2: f32, a1: f32, a2: f32,
    x1: f32, x2: f32, y1: f32, y2: f32,
    energy_removed_ema: f32,
    stats_buf: [(StatKey, f64); 1],
}

impl HighPassFilter {
    /// RBJ Audio EQ Cookbook: <https://www.w3.org/2011/audio/audio-eq-cookbook.html>
    pub fn new(cutoff_hz: f32, sample_rate: u32) -> Self {
        let (b0, b1, b2, a1, a2) = compute_hpf_coefficients(cutoff_hz, sample_rate);

        log::debug!(
            "HPF: cutoff={}Hz, rate={}Hz, coeffs=[{:.6}, {:.6}, {:.6}, {:.6}, {:.6}]",
            cutoff_hz, sample_rate, b0, b1, b2, a1, a2
        );

        Self {
            b0, b1, b2, a1, a2,
            x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0,
            energy_removed_ema: 0.0,
            stats_buf: [(StatKey::EnergyRemoved, 0.0)],
        }
    }
}

/// Normalized biquad coefficients (a0 = 1) for a second-order Butterworth HPF.
fn compute_hpf_coefficients(cutoff_hz: f32, sample_rate: u32) -> (f32, f32, f32, f32, f32) {
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate as f32;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2); // Q = 1/√2

    let a0 = 1.0 + alpha;
    let b0 = ((1.0 + cos_w0) / 2.0) / a0;
    let b1 = (-(1.0 + cos_w0)) / a0;
    let b2 = ((1.0 + cos_w0) / 2.0) / a0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;

    (b0, b1, b2, a1, a2)
}

impl Stage for HighPassFilter {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        if samples.is_empty() {
            return true;
        }

        let pre_energy: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();

        for s in samples.iter_mut() {
            let x0 = *s as f32;
            let y0 = self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2
                - self.a1 * self.y1
                - self.a2 * self.y2;

            self.x2 = self.x1;
            self.x1 = x0;
            self.y2 = self.y1;
            self.y1 = y0;

            *s = y0.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }

        let post_energy: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();

        let ratio = if pre_energy > 0 {
            1.0 - (post_energy as f32 / pre_energy as f32)
        } else {
            0.0
        };
        self.energy_removed_ema = self.energy_removed_ema * 0.95 + ratio * 0.05;
        self.stats_buf[0].1 = self.energy_removed_ema as f64;
        true
    }

    fn analyze(self, _pending: &mut StagePending) -> Self {
        self
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "highpass"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_wave(freq_hz: f32, sample_rate: u32, n_samples: usize, amplitude: i16) -> Vec<i16> {
        (0..n_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (amplitude as f32 * (2.0 * std::f32::consts::PI * freq_hz * t).sin()) as i16
            })
            .collect()
    }

    fn rms(samples: &[i16]) -> f64 {
        let sum_sq: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();
        ((sum_sq as f64) / samples.len() as f64).sqrt()
    }

    #[test]
    fn removes_dc_offset() {
        let mut hpf = HighPassFilter::new(85.0, 16000);
        let mut samples = vec![1000i16; 320];

        for _ in 0..20 {
            hpf.process(&mut samples);
            samples = vec![1000i16; 320];
        }

        hpf.process(&mut samples);
        let output_rms = rms(&samples);
        assert!(output_rms < 50.0, "DC offset should be removed, got RMS={:.1}", output_rms);
    }

    #[test]
    fn passes_speech_frequencies() {
        let mut hpf = HighPassFilter::new(85.0, 16000);
        let original = sine_wave(1000.0, 16000, 320, 10000);
        let original_rms = rms(&original);

        for _ in 0..10 {
            let mut frame = sine_wave(1000.0, 16000, 320, 10000);
            hpf.process(&mut frame);
        }

        let mut frame = original.clone();
        hpf.process(&mut frame);
        let retention = rms(&frame) / original_rms;
        assert!(retention > 0.90, "1kHz should pass through, retention={:.3}", retention);
    }

    #[test]
    fn attenuates_low_frequency() {
        let mut hpf = HighPassFilter::new(85.0, 16000);

        // Settle the filter
        for _ in 0..50 {
            let mut frame = sine_wave(30.0, 16000, 320, 10000);
            hpf.process(&mut frame);
        }

        let mut frame = sine_wave(30.0, 16000, 320, 10000);
        let pre_rms = rms(&frame);
        hpf.process(&mut frame);
        let post_rms = rms(&frame);

        // Theoretical ~87% but phase discontinuities between frames reduce it
        let attenuation = 1.0 - (post_rms / pre_rms);
        assert!(attenuation > 0.70, "30Hz: {:.1}% attenuation", attenuation * 100.0);
    }

    #[test]
    fn silence_stays_silent() {
        let mut hpf = HighPassFilter::new(85.0, 16000);
        let mut samples = vec![0i16; 320];
        hpf.process(&mut samples);
        assert!(samples.iter().all(|&s| s == 0));
    }

    #[test]
    fn stats_report_energy_removed() {
        let mut hpf = HighPassFilter::new(85.0, 16000);

        for _ in 0..20 {
            let mut samples = vec![5000i16; 320];
            hpf.process(&mut samples);
        }

        let stats = hpf.stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].0, StatKey::EnergyRemoved);
        assert!(stats[0].1 > 0.5, "DC removal ratio: {:.3}", stats[0].1);
    }

    #[test]
    fn analyze_returns_self() {
        let hpf = HighPassFilter::new(85.0, 16000);
        let hpf2 = hpf.analyze(&mut StagePending::default());
        assert_eq!(hpf2.name(), "highpass");
    }

    #[test]
    fn coefficients_are_sane() {
        let (b0, b1, b2, a1, a2) = compute_hpf_coefficients(85.0, 16000);

        assert!(b0.is_finite());
        assert!(b1.is_finite());
        assert!(b2.is_finite());
        assert!(a1.is_finite());
        assert!(a2.is_finite());

        // HPF symmetry: b0 == b2, b1 == -2*b0
        assert!((b0 - b2).abs() < 1e-6, "b0 ({}) != b2 ({})", b0, b2);
        assert!((b1 + 2.0 * b0).abs() < 1e-6, "b1 ({}) != -2*b0 ({})", b1, -2.0 * b0);
    }

    #[test]
    fn empty_frame_is_noop() {
        let mut hpf = HighPassFilter::new(85.0, 16000);
        let mut samples: Vec<i16> = vec![];
        hpf.process(&mut samples);
        assert!(samples.is_empty());
    }
}
