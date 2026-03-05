use super::{Stage, StagePending, StatKey};

/// Bandpass energy ratio observer.
///
/// Cascades a second-order HPF and LPF (both Butterworth biquads) to isolate
/// the speech band (default 300–3000 Hz) and reports the ratio of speech-band
/// energy to total energy. Observation-only: always returns `true` and does
/// **not** mutate the input samples — the biquad filters run on a read-only
/// copy path.
pub struct BandpassEnergy {
    hp: BiquadState,
    lp: BiquadState,
    speech_ratio_ema: f32,
    stats_buf: [(StatKey, f64); 1],
}

struct BiquadState {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    fn process_sample(&mut self, x0: f32) -> f32 {
        let y0 = self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }
}

/// RBJ Audio EQ Cookbook HPF coefficients (normalized, a0 = 1).
fn hpf_coefficients(cutoff_hz: f32, sample_rate: u32) -> BiquadState {
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate as f32;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2);

    let a0 = 1.0 + alpha;
    BiquadState {
        b0: ((1.0 + cos_w0) / 2.0) / a0,
        b1: (-(1.0 + cos_w0)) / a0,
        b2: ((1.0 + cos_w0) / 2.0) / a0,
        a1: (-2.0 * cos_w0) / a0,
        a2: (1.0 - alpha) / a0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

/// RBJ Audio EQ Cookbook LPF coefficients (normalized, a0 = 1).
fn lpf_coefficients(cutoff_hz: f32, sample_rate: u32) -> BiquadState {
    let w0 = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate as f32;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2);

    let a0 = 1.0 + alpha;
    BiquadState {
        b0: ((1.0 - cos_w0) / 2.0) / a0,
        b1: (1.0 - cos_w0) / a0,
        b2: ((1.0 - cos_w0) / 2.0) / a0,
        a1: (-2.0 * cos_w0) / a0,
        a2: (1.0 - alpha) / a0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

impl BandpassEnergy {
    pub fn new(low_hz: f32, high_hz: f32, sample_rate: u32) -> Self {
        log::info!(
            "Bandpass energy observer: {}-{}Hz, rate={}Hz",
            low_hz, high_hz, sample_rate
        );
        Self {
            hp: hpf_coefficients(low_hz, sample_rate),
            lp: lpf_coefficients(high_hz, sample_rate),
            speech_ratio_ema: 0.0,
            stats_buf: [(StatKey::SpeechBandRatio, 0.0)],
        }
    }
}

impl Stage for BandpassEnergy {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        if samples.is_empty() {
            return true;
        }

        let mut total_energy: f64 = 0.0;
        let mut band_energy: f64 = 0.0;

        for &s in samples.iter() {
            let x = s as f32;
            total_energy += (x as f64) * (x as f64);

            // HPF → LPF cascade (read-only path, does not touch samples)
            let hp_out = self.hp.process_sample(x);
            let bp_out = self.lp.process_sample(hp_out);
            band_energy += (bp_out as f64) * (bp_out as f64);
        }

        let ratio = if total_energy > 0.0 {
            (band_energy / total_energy) as f32
        } else {
            0.0
        };

        self.speech_ratio_ema = self.speech_ratio_ema * 0.9 + ratio * 0.1;
        self.stats_buf[0].1 = self.speech_ratio_ema as f64;

        true // observation-only
    }

    fn analyze(self, _pending: &mut StagePending) -> Self {
        self
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "bandpass"
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

    #[test]
    fn speech_frequency_has_high_ratio() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);

        // Settle the filter with 1kHz (well inside speech band)
        for _ in 0..50 {
            let mut frame = sine_wave(1000.0, 16000, 320, 10000);
            bp.process(&mut frame);
        }

        let ratio = bp.stats()[0].1;
        assert!(
            ratio > 0.7,
            "1kHz speech-band ratio should be high, got {:.3}",
            ratio
        );
    }

    #[test]
    fn sub_bass_has_low_ratio() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);

        // Settle with 50Hz (well below speech band)
        for _ in 0..100 {
            let mut frame = sine_wave(50.0, 16000, 320, 10000);
            bp.process(&mut frame);
        }

        let ratio = bp.stats()[0].1;
        assert!(
            ratio < 0.3,
            "50Hz should have low speech-band ratio, got {:.3}",
            ratio
        );
    }

    #[test]
    fn above_band_has_low_ratio() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);

        // Settle with 6kHz (above speech band)
        for _ in 0..100 {
            let mut frame = sine_wave(6000.0, 16000, 320, 10000);
            bp.process(&mut frame);
        }

        let ratio = bp.stats()[0].1;
        assert!(
            ratio < 0.3,
            "6kHz should have low speech-band ratio, got {:.3}",
            ratio
        );
    }

    #[test]
    fn does_not_mutate_samples() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        let original = sine_wave(1000.0, 16000, 320, 10000);
        let mut samples = original.clone();
        bp.process(&mut samples);
        assert_eq!(samples, original, "bandpass must not mutate input samples");
    }

    #[test]
    fn silence_returns_true() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        let mut samples = vec![0i16; 320];
        assert!(bp.process(&mut samples));
    }

    #[test]
    fn empty_frame_returns_true() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        let mut samples: Vec<i16> = vec![];
        assert!(bp.process(&mut samples));
    }

    #[test]
    fn always_returns_true() {
        let mut bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        let mut loud = sine_wave(500.0, 16000, 320, 10000);
        assert!(bp.process(&mut loud));
    }

    #[test]
    fn name_is_bandpass() {
        let bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        assert_eq!(bp.name(), "bandpass");
    }

    #[test]
    fn analyze_returns_self() {
        let bp = BandpassEnergy::new(300.0, 3000.0, 16000);
        let bp2 = bp.analyze(&mut StagePending::default());
        assert_eq!(bp2.name(), "bandpass");
    }
}
