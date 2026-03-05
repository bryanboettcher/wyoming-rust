use super::{Stage, StagePending, StatKey};

/// Compute RMS from i16 samples. Returns u16 in range 0-32768.
pub fn compute_rms(samples: &[i16]) -> u16 {
    if samples.is_empty() {
        return 0;
    }
    let sum_of_squares: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();
    let mean_square = sum_of_squares / samples.len() as i64;
    (mean_square as f64).sqrt().min(32768.0) as u16
}

/// Static threshold Schmitt trigger energy detector.
///
/// Uses separate attack (onset) and sustain (hold) thresholds for
/// hysteresis-based voice activity detection. Once RMS crosses the
/// attack threshold, detection switches to the lower sustain threshold,
/// preventing rapid toggling near the boundary.
pub struct EnergyDetector {
    attack_threshold: u16,
    sustain_threshold: u16,
    last_rms: u16,
    sustaining: bool,
    stats_buf: [(StatKey, f64); 5],
}

impl EnergyDetector {
    pub fn new(attack_threshold: u16, sustain_threshold: u16) -> Self {
        let effective_sustain = if sustain_threshold == 0 {
            attack_threshold / 2
        } else {
            sustain_threshold
        };
        if effective_sustain >= attack_threshold {
            log::warn!(
                "Sustain threshold ({}) >= attack threshold ({}); hysteresis will have no effect",
                effective_sustain, attack_threshold
            );
        }
        log::info!(
            "Energy detector: attack={}, sustain={}",
            attack_threshold, effective_sustain
        );
        Self {
            attack_threshold,
            sustain_threshold: effective_sustain,
            last_rms: 0,
            sustaining: false,
            stats_buf: [
                (StatKey::Energy, 0.0),
                (StatKey::AttackThreshold, attack_threshold as f64),
                (StatKey::SustainThreshold, effective_sustain as f64),
                (StatKey::Triggered, 0.0),
                (StatKey::Phase, 0.0),
            ],
        }
    }
}

impl Stage for EnergyDetector {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        let rms = compute_rms(samples);
        self.last_rms = rms;

        let active_threshold = if self.sustaining {
            self.sustain_threshold
        } else {
            self.attack_threshold
        };

        let triggered = rms >= active_threshold;

        // Bidirectional Schmitt trigger: enter sustain on onset, exit when
        // signal drops below the sustain threshold.
        if triggered && !self.sustaining {
            self.sustaining = true;
            log::debug!("Energy detector phase: attack -> sustain (rms={})", rms);
        } else if !triggered && self.sustaining {
            self.sustaining = false;
            log::debug!("Energy detector phase: sustain -> attack (rms={})", rms);
        }

        self.stats_buf[0].1 = rms as f64;
        self.stats_buf[3].1 = if triggered { 1.0 } else { 0.0 };
        self.stats_buf[4].1 = if self.sustaining { 1.0 } else { 0.0 };

        triggered
    }

    fn analyze(self, _pending: &mut StagePending) -> Self {
        self
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "energy_detector"
    }

    fn reset(&mut self) {
        self.last_rms = 0;
        self.sustaining = false;
        self.stats_buf[3].1 = 0.0;
        self.stats_buf[4].1 = 0.0;
    }
}

/// Adaptive energy detector with automatic noise floor tracking.
///
/// Maintains an exponential moving average of the ambient noise floor
/// and derives attack/sustain thresholds as ratios of that floor.
/// Uses a two-phase EMA alpha: fast warmup (first 100 samples) for
/// quick convergence, then slow tracking to resist drift.
///
/// The noise floor is only updated when not in sustain phase and when
/// the current RMS is below the attack threshold, preventing speech
/// energy from inflating the floor estimate.
pub struct AutoEnergyDetector {
    attack_ratio: f32,
    sustain_ratio: f32,
    noise_floor: f32,
    noise_floor_samples: u32,
    last_rms: u16,
    sustaining: bool,
    hold_frames: u16,
    hold_counter: u16,
    stats_buf: [(StatKey, f64); 7],
}

impl AutoEnergyDetector {
    pub fn new(attack_ratio: f32, sustain_ratio: f32, hold_frames: u16) -> Self {
        log::info!(
            "Auto energy detector: attack_ratio={}, sustain_ratio={}, hold_frames={}",
            attack_ratio, sustain_ratio, hold_frames
        );
        Self {
            attack_ratio,
            sustain_ratio,
            noise_floor: 0.0,
            noise_floor_samples: 0,
            last_rms: 0,
            sustaining: false,
            hold_frames,
            hold_counter: 0,
            stats_buf: [
                (StatKey::Energy, 0.0),
                (StatKey::NoiseFloor, 0.0),
                (StatKey::AttackThreshold, 50.0),
                (StatKey::SustainThreshold, 50.0),
                (StatKey::Triggered, 0.0),
                (StatKey::Phase, 0.0),
                (StatKey::HoldCounter, 0.0),
            ],
        }
    }

    fn effective_attack(&self) -> u16 {
        (self.noise_floor * self.attack_ratio).max(50.0) as u16
    }

    fn effective_sustain(&self) -> u16 {
        (self.noise_floor * self.sustain_ratio).max(50.0) as u16
    }

    fn update_noise_floor(&mut self, rms: u16, triggered: bool) {
        // Seed with first sample unconditionally (bootstrap)
        if self.noise_floor_samples == 0 {
            self.noise_floor = rms as f32;
            self.noise_floor_samples = 1;
            return;
        }

        // Only update floor when not actively triggered — prevents speech
        // and loud transients from inflating the floor estimate. With
        // bidirectional Schmitt trigger, the floor adapts between utterances
        // and tracks changing ambient levels.
        if triggered {
            return;
        }

        self.noise_floor_samples += 1;
        // Two-phase alpha: fast warmup, slow tracking
        let alpha = if self.noise_floor_samples <= 100 {
            0.05
        } else {
            0.002
        };

        self.noise_floor = self.noise_floor * (1.0 - alpha) + rms as f32 * alpha;
    }
}

impl Stage for AutoEnergyDetector {
    fn process(&mut self, samples: &mut [i16]) -> bool {
        let rms = compute_rms(samples);
        self.last_rms = rms;

        let attack = self.effective_attack();
        let sustain = self.effective_sustain();

        let active_threshold = if self.sustaining { sustain } else { attack };
        let triggered = rms >= active_threshold;

        // Bidirectional Schmitt trigger: enter sustain on onset, exit when
        // signal drops below the sustain threshold. This allows the noise
        // floor to adapt between utterances rather than freezing permanently.
        if triggered && !self.sustaining {
            self.sustaining = true;
            log::debug!(
                "Auto energy phase: attack -> sustain (rms={}, floor={:.0})",
                rms, self.noise_floor
            );
        } else if !triggered && self.sustaining {
            self.sustaining = false;
            log::debug!(
                "Auto energy phase: sustain -> attack (rms={}, floor={:.0})",
                rms, self.noise_floor
            );
        }

        // Update noise floor AFTER trigger decision so it uses current state
        self.update_noise_floor(rms, triggered);

        // Hold: require hold_frames consecutive triggered frames before
        // reporting true. Suppresses sub-second transient false triggers.
        // The preroll buffer (default 1s = 50 frames) captures audio before
        // the hold fires, so no speech onset is lost.
        if triggered {
            self.hold_counter = self.hold_counter.saturating_add(1);
        } else {
            self.hold_counter = 0;
        }
        let output = if self.hold_frames == 0 {
            triggered
        } else {
            self.hold_counter >= self.hold_frames
        };

        self.stats_buf[0].1 = rms as f64;
        self.stats_buf[1].1 = self.noise_floor as f64;
        self.stats_buf[2].1 = attack as f64;
        self.stats_buf[3].1 = sustain as f64;
        self.stats_buf[4].1 = if output { 1.0 } else { 0.0 };
        self.stats_buf[5].1 = if self.sustaining { 1.0 } else { 0.0 };
        self.stats_buf[6].1 = self.hold_counter as f64;

        output
    }

    fn analyze(mut self, pending: &mut StagePending) -> Self {
        if let Some(ratio) = pending.attack_ratio.take() {
            log::info!("Auto energy: attack_ratio updated to {}", ratio);
            self.attack_ratio = ratio;
        }
        if let Some(ratio) = pending.sustain_ratio.take() {
            log::info!("Auto energy: sustain_ratio updated to {}", ratio);
            self.sustain_ratio = ratio;
        }
        self
    }

    fn stats(&self) -> &[(StatKey, f64)] {
        &self.stats_buf
    }

    fn name(&self) -> &'static str {
        "auto_energy"
    }

    fn reset(&mut self) {
        // Preserve noise_floor and noise_floor_samples across resets
        self.last_rms = 0;
        self.sustaining = false;
        self.hold_counter = 0;
        self.stats_buf[4].1 = 0.0;
        self.stats_buf[5].1 = 0.0;
        self.stats_buf[6].1 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: create a uniform i16 sample buffer
    fn make_samples(value: i16, count: usize) -> Vec<i16> {
        vec![value; count]
    }

    // ========== compute_rms tests ==========

    #[test]
    fn rms_silence() {
        assert_eq!(compute_rms(&make_samples(0, 320)), 0);
    }

    #[test]
    fn rms_uniform_value() {
        assert_eq!(compute_rms(&make_samples(1000, 320)), 1000);
    }

    #[test]
    fn rms_empty() {
        assert_eq!(compute_rms(&[]), 0);
    }

    // ========== EnergyDetector tests ==========

    #[test]
    fn energy_detector_silence_no_trigger() {
        let mut det = EnergyDetector::new(1000, 500);
        let mut silence = make_samples(0, 320);
        assert!(!det.process(&mut silence));
    }

    #[test]
    fn energy_detector_loud_frame_triggers() {
        let mut det = EnergyDetector::new(1000, 500);
        let mut loud = make_samples(10000, 320);
        assert!(det.process(&mut loud));
    }

    #[test]
    fn energy_detector_threshold_boundary() {
        let mut det = EnergyDetector::new(5000, 2500);
        let mut frame = make_samples(5000, 320);
        assert!(det.process(&mut frame)); // RMS=5000 >= attack=5000
    }

    #[test]
    fn energy_detector_sustain_default() {
        let det = EnergyDetector::new(1000, 0);
        // sustain=0 resolves to attack/2 = 500
        assert_eq!(det.stats_buf[2].1, 500.0); // SustainThreshold
    }

    #[test]
    fn energy_detector_schmitt_trigger_phases() {
        let mut det = EnergyDetector::new(500, 200);

        // Initial phase is attack (0.0)
        let stats = det.stats();
        assert_eq!(stats[4].1, 0.0); // Phase = attack

        // RMS 300: below attack (500) -> no trigger
        let mut mid = make_samples(300, 320);
        assert!(!det.process(&mut mid));
        assert_eq!(det.stats()[4].1, 0.0); // Still attack

        // RMS 600: above attack -> triggers, enters sustain
        let mut loud = make_samples(600, 320);
        assert!(det.process(&mut loud));
        assert_eq!(det.stats()[4].1, 1.0); // Phase = sustain

        // RMS 300: above sustain (200) -> still triggers in sustain
        assert!(det.process(&mut mid));
        assert_eq!(det.stats()[4].1, 1.0); // Still sustain

        // RMS 100: below sustain -> not triggered, exits sustain
        let mut quiet = make_samples(100, 320);
        assert!(!det.process(&mut quiet));
        assert_eq!(det.stats()[4].1, 0.0); // Back to attack (bidirectional)

        // RMS 300: below attack -> doesn't trigger in attack phase
        assert!(!det.process(&mut mid));
        assert_eq!(det.stats()[4].1, 0.0); // Still attack
    }

    // ========== AutoEnergyDetector tests ==========

    #[test]
    fn auto_energy_noise_floor_converges() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);

        // Feed 100 frames at RMS=100 (warmup phase).
        // First frame seeds floor=100, attack=300. RMS 100 < 300, so not
        // triggered, and floor updates normally via EMA.
        for _ in 0..100 {
            let mut frame = make_samples(100, 320);
            det.process(&mut frame);
        }

        // Noise floor should be near 100
        let floor = det.noise_floor;
        assert!(
            floor > 80.0 && floor < 120.0,
            "noise floor should converge near 100, got {:.1}",
            floor
        );
    }

    #[test]
    fn auto_energy_auto_thresholds() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        // Manually set floor to known value
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;

        assert_eq!(det.effective_attack(), 300); // 100 * 3.0
        assert_eq!(det.effective_sustain(), 150); // 100 * 1.5
    }

    #[test]
    fn auto_energy_noise_floor_ignores_speech() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        // Set a known floor
        det.noise_floor = 50.0;
        det.noise_floor_samples = 200;
        let floor_before = det.noise_floor;

        // Trigger with loud frame (enters sustain)
        let mut loud = make_samples(1000, 320);
        det.process(&mut loud);

        // Floor should not change during sustain
        assert_eq!(det.noise_floor, floor_before);
    }

    #[test]
    fn auto_energy_noise_floor_survives_reset() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        det.noise_floor = 75.0;
        det.noise_floor_samples = 150;

        det.reset();

        assert_eq!(det.noise_floor, 75.0);
        assert_eq!(det.noise_floor_samples, 150);
        assert!(!det.sustaining);
        assert_eq!(det.last_rms, 0);
    }

    #[test]
    fn auto_energy_min_threshold() {
        let det = AutoEnergyDetector::new(3.0, 1.5, 0);
        // Floor at 0 -> thresholds clamp to 50
        assert_eq!(det.effective_attack(), 50);
        assert_eq!(det.effective_sustain(), 50);
    }

    #[test]
    fn auto_energy_analyze_applies_ratios() {
        let det = AutoEnergyDetector::new(3.0, 1.5, 0);
        let mut pending = StagePending {
            attack_ratio: Some(4.0),
            sustain_ratio: Some(2.0),
        };
        let det = det.analyze(&mut pending);

        assert_eq!(det.attack_ratio, 4.0);
        assert_eq!(det.sustain_ratio, 2.0);
        // Pending values should be consumed
        assert!(pending.attack_ratio.is_none());
        assert!(pending.sustain_ratio.is_none());
    }

    #[test]
    fn auto_energy_schmitt_exits_sustain() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        // Set a known floor so thresholds are predictable
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;
        // attack=300, sustain=150

        // Loud frame triggers and enters sustain
        let mut loud = make_samples(400, 320);
        assert!(det.process(&mut loud));
        assert!(det.sustaining);

        // Mid-level frame: above sustain (150) -> still triggered
        let mut mid = make_samples(200, 320);
        assert!(det.process(&mut mid));
        assert!(det.sustaining);

        // Quiet frame: below sustain (150) -> exits sustain
        let mut quiet = make_samples(80, 320);
        assert!(!det.process(&mut quiet));
        assert!(!det.sustaining); // Bidirectional: back to attack phase

        // Mid-level frame: below attack (300) -> doesn't trigger in attack phase
        assert!(!det.process(&mut mid));
    }

    #[test]
    fn auto_energy_floor_tracks_upward() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        // Simulate overnight: floor adapted to quiet environment
        det.noise_floor = 50.0;
        det.noise_floor_samples = 200;
        let initial_floor = det.noise_floor;

        // Feed ambient frames at RMS=100 (above old floor, below attack=150)
        for _ in 0..500 {
            let mut frame = make_samples(100, 320);
            det.process(&mut frame);
        }

        // Floor should have tracked upward toward 100
        assert!(
            det.noise_floor > initial_floor + 20.0,
            "floor should track upward from {:.0}, got {:.0}",
            initial_floor,
            det.noise_floor
        );
    }

    // ========== Hold counter tests ==========

    #[test]
    fn hold_suppresses_short_triggers() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 5);
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;
        // attack=300, sustain=150

        // 4 loud frames: raw triggers but hold not met
        for _ in 0..4 {
            let mut loud = make_samples(400, 320);
            assert!(!det.process(&mut loud), "hold should suppress until 5 frames");
        }

        // 5th frame: hold met
        let mut loud = make_samples(400, 320);
        assert!(det.process(&mut loud), "5th frame should pass hold");
    }

    #[test]
    fn hold_counter_resets_on_silence() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 5);
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;

        // 3 loud frames
        for _ in 0..3 {
            let mut loud = make_samples(400, 320);
            det.process(&mut loud);
        }
        assert_eq!(det.hold_counter, 3);

        // Silence resets counter
        let mut quiet = make_samples(50, 320);
        det.process(&mut quiet);
        assert_eq!(det.hold_counter, 0);
    }

    #[test]
    fn hold_zero_disables_hold() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;

        // Single loud frame triggers immediately
        let mut loud = make_samples(400, 320);
        assert!(det.process(&mut loud));
    }

    #[test]
    fn hold_counter_in_stats() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 5);
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;

        let mut loud = make_samples(400, 320);
        det.process(&mut loud);

        let stats = det.stats();
        assert_eq!(stats[6].0, StatKey::HoldCounter);
        assert_eq!(stats[6].1, 1.0);
    }

    #[test]
    fn hold_counter_reset_clears() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 5);
        det.noise_floor = 100.0;
        det.noise_floor_samples = 200;

        for _ in 0..3 {
            let mut loud = make_samples(400, 320);
            det.process(&mut loud);
        }
        assert_eq!(det.hold_counter, 3);

        det.reset();
        assert_eq!(det.hold_counter, 0);
        assert_eq!(det.stats()[6].1, 0.0);
    }

    #[test]
    fn auto_energy_floor_adapts_between_utterances() {
        let mut det = AutoEnergyDetector::new(3.0, 1.5, 0);
        det.noise_floor = 50.0;
        det.noise_floor_samples = 200;
        // attack=150, sustain=75

        // Loud frame triggers
        let mut loud = make_samples(200, 320);
        assert!(det.process(&mut loud));
        assert!(det.sustaining);
        let floor_during_speech = det.noise_floor;

        // Floor frozen during triggered frames
        assert_eq!(floor_during_speech, 50.0);

        // Quiet frame: drops below sustain, exits sustain
        let mut ambient = make_samples(60, 320);
        assert!(!det.process(&mut ambient));
        assert!(!det.sustaining);

        // Now floor can update (not triggered anymore)
        let floor_after = det.noise_floor;
        // With alpha=0.002 and one frame, change is tiny but positive
        // floor was 50.0, rms=60 -> floor should nudge upward
        assert!(
            floor_after >= floor_during_speech,
            "floor should be able to update between utterances"
        );
    }
}
