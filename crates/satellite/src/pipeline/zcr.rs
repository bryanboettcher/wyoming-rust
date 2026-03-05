use super::{Stage, StagePending, StatKey};

/// Zero-crossing rate observer.
///
/// Counts sign changes per frame and reports crossings/sec. Speech typically
/// produces 1000–3000 crossings/sec; broadband noise is higher, low rumble
/// is lower. Observation-only: always returns `true` and does not mutate
/// samples.
pub struct ZeroCrossingRate {
    sample_rate: u32,
    stats_buf: [(StatKey, f64); 1],
}

impl ZeroCrossingRate {
    pub fn new(sample_rate: u32) -> Self {
        log::info!("ZCR observer: sample_rate={}Hz", sample_rate);
        Self {
            sample_rate,
            stats_buf: [(StatKey::ZeroCrossingRate, 0.0)],
        }
    }
}

impl Stage for ZeroCrossingRate {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        if samples.len() < 2 {
            self.stats_buf[0].1 = 0.0;
            return true;
        }

        let crossings: u32 = samples
            .windows(2)
            .map(|w| {
                // Count a crossing when samples are on opposite sides of zero.
                // Treat zero as non-negative so exact-zero crossing points
                // (common with integer sine waves) still register.
                if (w[0] >= 0) != (w[1] >= 0) {
                    1u32
                } else {
                    0
                }
            })
            .sum();

        let zcr_per_sec =
            crossings as f64 * self.sample_rate as f64 / (samples.len() - 1) as f64;
        self.stats_buf[0].1 = zcr_per_sec;

        true // observation-only
    }

    fn analyze(self, _pending: &mut StagePending) -> Self {
        self
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "zcr"
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
    fn sine_zcr_approximates_twice_frequency() {
        let mut zcr = ZeroCrossingRate::new(16000);
        // 1kHz sine: ~2000 crossings/sec
        let mut samples = sine_wave(1000.0, 16000, 320, 10000);
        zcr.process(&mut samples);

        let rate = zcr.stats()[0].1;
        assert!(
            rate > 1800.0 && rate < 2200.0,
            "1kHz sine ZCR should be ~2000, got {:.0}",
            rate
        );
    }

    #[test]
    fn silence_has_zero_zcr() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let mut samples = vec![0i16; 320];
        zcr.process(&mut samples);

        assert_eq!(zcr.stats()[0].1, 0.0);
    }

    #[test]
    fn positive_dc_offset_has_zero_zcr() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let mut samples = vec![1000i16; 320];
        zcr.process(&mut samples);

        assert_eq!(zcr.stats()[0].1, 0.0);
    }

    #[test]
    fn negative_dc_offset_has_zero_zcr() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let mut samples = vec![-1000i16; 320];
        zcr.process(&mut samples);

        assert_eq!(zcr.stats()[0].1, 0.0);
    }

    #[test]
    fn alternating_signal_has_max_zcr() {
        let mut zcr = ZeroCrossingRate::new(16000);
        // +1, -1, +1, -1, ... → crossing every sample pair
        let mut samples: Vec<i16> = (0..320).map(|i| if i % 2 == 0 { 1000 } else { -1000 }).collect();
        zcr.process(&mut samples);

        let rate = zcr.stats()[0].1;
        // 319 crossings in 319 gaps → rate = sample_rate
        let expected = 16000.0;
        assert!(
            (rate - expected).abs() < 100.0,
            "alternating signal ZCR should be ~{}, got {:.0}",
            expected,
            rate
        );
    }

    #[test]
    fn always_returns_true() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let mut samples = vec![0i16; 320];
        assert!(zcr.process(&mut samples));

        let mut loud = sine_wave(500.0, 16000, 320, 10000);
        assert!(zcr.process(&mut loud));
    }

    #[test]
    fn does_not_mutate_samples() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let original = sine_wave(1000.0, 16000, 320, 10000);
        let mut samples = original.clone();
        zcr.process(&mut samples);
        assert_eq!(samples, original);
    }

    #[test]
    fn short_frame_handled() {
        let mut zcr = ZeroCrossingRate::new(16000);
        let mut one = vec![100i16];
        assert!(zcr.process(&mut one));
        assert_eq!(zcr.stats()[0].1, 0.0);

        let mut empty: Vec<i16> = vec![];
        assert!(zcr.process(&mut empty));
        assert_eq!(zcr.stats()[0].1, 0.0);
    }

    #[test]
    fn name_is_zcr() {
        let zcr = ZeroCrossingRate::new(16000);
        assert_eq!(zcr.name(), "zcr");
    }

    #[test]
    fn analyze_returns_self() {
        let zcr = ZeroCrossingRate::new(16000);
        let zcr2 = zcr.analyze(&mut StagePending::default());
        assert_eq!(zcr2.name(), "zcr");
    }
}
