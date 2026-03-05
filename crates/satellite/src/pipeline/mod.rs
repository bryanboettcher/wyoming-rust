pub mod bandpass;
pub mod energy;
pub mod hpf;
pub mod mfcc;
pub mod zcr;

use std::time::Instant;

use crate::config::PipelineConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatKey {
    NoiseFloor,
    DutyCycle,
    CurrentGain,
    ClippingCount,
    EnergyRemoved,
    ProcessUs,
    AnalyzeUs,
    Energy,
    AttackThreshold,
    SustainThreshold,
    Triggered,
    Phase,
    HoldCounter,
    ZeroCrossingRate,
    SpeechBandRatio,
    SpectralCentroid,
    SpectralFlatness,
}

impl StatKey {
    pub fn name(self) -> &'static str {
        match self {
            StatKey::NoiseFloor => "noise_floor",
            StatKey::DutyCycle => "duty_cycle",
            StatKey::CurrentGain => "agc_gain",
            StatKey::ClippingCount => "clipping_count",
            StatKey::EnergyRemoved => "energy_removed",
            StatKey::ProcessUs => "process_us",
            StatKey::AnalyzeUs => "analyze_us",
            StatKey::Energy => "energy",
            StatKey::AttackThreshold => "attack_threshold",
            StatKey::SustainThreshold => "sustain_threshold",
            StatKey::Triggered => "triggered",
            StatKey::Phase => "phase",
            StatKey::HoldCounter => "hold_counter",
            StatKey::ZeroCrossingRate => "zcr",
            StatKey::SpeechBandRatio => "speech_band_ratio",
            StatKey::SpectralCentroid => "spectral_centroid",
            StatKey::SpectralFlatness => "spectral_flatness",
        }
    }
}

/// Runtime config changes from HTTP, consumed by stages during `analyze()`.
#[derive(Default)]
pub struct StagePending {
    pub attack_ratio: Option<f32>,
    pub sustain_ratio: Option<f32>,
}

pub trait Stage: Send {
    /// Process audio samples. Returns `true` to continue pipeline, `false` to short-circuit.
    /// Filters always return `true`; detectors return their detection result.
    fn process(&mut self, samples: &mut [i16]) -> bool;
    fn analyze(self, pending: &mut StagePending) -> Self where Self: Sized;
    fn stats(&self) -> &[(StatKey, f64)];
    fn name(&self) -> &'static str;
    fn reset(&mut self) {}
}

pub enum StageKind {
    HighPass(hpf::HighPassFilter),
    Bandpass(bandpass::BandpassEnergy),
    Zcr(zcr::ZeroCrossingRate),
    Mfcc(mfcc::MfccStage),
    EnergyDetector(energy::EnergyDetector),
    AutoEnergy(energy::AutoEnergyDetector),
}

impl StageKind {
    pub fn process(&mut self, samples: &mut [i16]) -> bool {
        match self {
            Self::HighPass(s) => s.process(samples),
            Self::Bandpass(s) => s.process(samples),
            Self::Zcr(s) => s.process(samples),
            Self::Mfcc(s) => s.process(samples),
            Self::EnergyDetector(s) => s.process(samples),
            Self::AutoEnergy(s) => s.process(samples),
        }
    }

    pub fn analyze(self, pending: &mut StagePending) -> Self {
        match self {
            Self::HighPass(s) => Self::HighPass(s.analyze(pending)),
            Self::Bandpass(s) => Self::Bandpass(s.analyze(pending)),
            Self::Zcr(s) => Self::Zcr(s.analyze(pending)),
            Self::Mfcc(s) => Self::Mfcc(s.analyze(pending)),
            Self::EnergyDetector(s) => Self::EnergyDetector(s.analyze(pending)),
            Self::AutoEnergy(s) => Self::AutoEnergy(s.analyze(pending)),
        }
    }

    pub fn stats(&self) -> &[(StatKey, f64)] {
        match self {
            Self::HighPass(s) => s.stats(),
            Self::Bandpass(s) => s.stats(),
            Self::Zcr(s) => s.stats(),
            Self::Mfcc(s) => s.stats(),
            Self::EnergyDetector(s) => s.stats(),
            Self::AutoEnergy(s) => s.stats(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::HighPass(s) => s.name(),
            Self::Bandpass(s) => s.name(),
            Self::Zcr(s) => s.name(),
            Self::Mfcc(s) => s.name(),
            Self::EnergyDetector(s) => s.name(),
            Self::AutoEnergy(s) => s.name(),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::HighPass(s) => s.reset(),
            Self::Bandpass(s) => s.reset(),
            Self::Zcr(s) => s.reset(),
            Self::Mfcc(s) => s.reset(),
            Self::EnergyDetector(s) => s.reset(),
            Self::AutoEnergy(s) => s.reset(),
        }
    }

    /// Returns quantized mel-band energies if this is an MFCC stage.
    pub fn mel_energies(&self) -> Option<&[u8]> {
        match self {
            Self::Mfcc(s) => Some(s.mel_bytes()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatsSnapshot {
    pub entries: Vec<(&'static str, StatKey, f64)>,
    /// Quantized log-mel energies (u8, 0-255) from MFCC stage, if present.
    pub mel_energies: Option<Vec<u8>>,
}

pub trait Runner: Send {
    /// Process one audio frame through all stages. Returns `true` if detection passed
    /// (all stages returned true), `false` if any detector rejected the frame.
    fn process_frame(&mut self, samples: &mut [i16]) -> bool;
    fn reset(&mut self);
    fn stats_snapshot(&self) -> StatsSnapshot;
    fn pending(&mut self) -> &mut StagePending;
    fn stage_names(&self) -> Vec<&'static str>;
}

pub struct SimpleRunner {
    stages: Vec<StageKind>,
    frame_count: u64,
    analyze_interval: u64,
    stage_timings_us: Vec<u32>,
    last_analyze_us: u32,
    pending: StagePending,
}

impl SimpleRunner {
    pub fn new(config: &PipelineConfig, sample_rate: u32) -> Self {
        let mut stages = Vec::new();

        // Filters first
        if let Some(ref hpf_config) = config.highpass {
            if hpf_config.enabled {
                stages.push(StageKind::HighPass(
                    hpf::HighPassFilter::new(hpf_config.cutoff_hz, sample_rate),
                ));
            }
        }

        // Observers second (read-only, always return true)
        if let Some(ref bp_config) = config.bandpass {
            if bp_config.enabled {
                stages.push(StageKind::Bandpass(
                    bandpass::BandpassEnergy::new(bp_config.low_hz, bp_config.high_hz, sample_rate),
                ));
            }
        }
        if let Some(ref zcr_config) = config.zcr {
            if zcr_config.enabled {
                stages.push(StageKind::Zcr(
                    zcr::ZeroCrossingRate::new(sample_rate),
                ));
            }
        }
        if let Some(ref mfcc_config) = config.mfcc {
            if mfcc_config.enabled {
                let frame_size = (sample_rate * 20 / 1000) as usize; // 320 at 16kHz
                stages.push(StageKind::Mfcc(
                    mfcc::MfccStage::new(
                        mfcc_config.n_mfcc,
                        mfcc_config.n_mels,
                        mfcc_config.n_fft,
                        frame_size,
                        sample_rate,
                    ),
                ));
            }
        }

        // Detectors last (short-circuit on first false)
        if let Some(ref energy_config) = config.energy_detector {
            stages.push(StageKind::EnergyDetector(
                energy::EnergyDetector::new(
                    energy_config.attack_threshold,
                    energy_config.sustain_threshold,
                ),
            ));
        }
        if let Some(ref auto_config) = config.auto_energy {
            stages.push(StageKind::AutoEnergy(
                energy::AutoEnergyDetector::new(
                    auto_config.attack_ratio,
                    auto_config.sustain_ratio,
                    auto_config.hold_frames,
                ),
            ));
        }

        let analyze_interval = if config.analyze_interval == 0 {
            50
        } else {
            config.analyze_interval
        };

        log::info!(
            "Audio pipeline: {} stage(s), analyze every {} frames",
            stages.len(),
            analyze_interval,
        );
        for stage in &stages {
            log::info!("  - {}", stage.name());
        }

        let stage_count = stages.len();
        Self {
            stages,
            frame_count: 0,
            analyze_interval,
            stage_timings_us: vec![0; stage_count],
            last_analyze_us: 0,
            pending: StagePending::default(),
        }
    }

}

impl Runner for SimpleRunner {
    fn process_frame(&mut self, samples: &mut [i16]) -> bool {
        let mut result = true;
        for (i, stage) in self.stages.iter_mut().enumerate() {
            let t = Instant::now();
            if result {
                result = stage.process(samples);
            }
            self.stage_timings_us[i] = t.elapsed().as_micros() as u32;
        }
        self.frame_count += 1;
        if self.frame_count % self.analyze_interval == 0 {
            let t = Instant::now();
            self.stages = self.stages.drain(..).map(|s| s.analyze(&mut self.pending)).collect();
            self.last_analyze_us = t.elapsed().as_micros() as u32;
        }
        result
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }

    fn stats_snapshot(&self) -> StatsSnapshot {
        let mut entries = Vec::new();
        let mut mel_energies = None;
        for (i, stage) in self.stages.iter().enumerate() {
            for &(key, val) in stage.stats() {
                entries.push((stage.name(), key, val));
            }
            if let Some(&us) = self.stage_timings_us.get(i) {
                entries.push((stage.name(), StatKey::ProcessUs, us as f64));
            }
            if mel_energies.is_none() {
                if let Some(mel) = stage.mel_energies() {
                    mel_energies = Some(mel.to_vec());
                }
            }
        }
        if self.last_analyze_us > 0 {
            entries.push(("runner", StatKey::AnalyzeUs, self.last_analyze_us as f64));
        }
        StatsSnapshot { entries, mel_energies }
    }

    fn pending(&mut self) -> &mut StagePending {
        &mut self.pending
    }

    fn stage_names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.name()).collect()
    }
}

pub fn pcm16le_to_samples(bytes: &[u8], out: &mut Vec<i16>) {
    out.clear();
    out.extend(bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])));
}

pub fn samples_to_pcm16le(samples: &[i16], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(samples.len() * 2);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16le_round_trip() {
        let original: Vec<i16> = vec![0, 1, -1, i16::MAX, i16::MIN, 12345, -12345];
        let mut bytes = Vec::new();
        samples_to_pcm16le(&original, &mut bytes);
        assert_eq!(bytes.len(), original.len() * 2);

        let mut recovered = Vec::new();
        pcm16le_to_samples(&bytes, &mut recovered);
        assert_eq!(recovered, original);
    }

    #[test]
    fn pcm16le_empty() {
        let mut bytes = Vec::new();
        samples_to_pcm16le(&[], &mut bytes);
        assert!(bytes.is_empty());

        let mut samples = Vec::new();
        pcm16le_to_samples(&[], &mut samples);
        assert!(samples.is_empty());
    }

    #[test]
    fn pcm16le_reuses_buffer() {
        let samples: Vec<i16> = vec![100, 200, 300];
        let mut bytes = Vec::with_capacity(100);
        samples_to_pcm16le(&samples, &mut bytes);
        assert_eq!(bytes.len(), 6);
        assert!(bytes.capacity() >= 100);
    }

    #[test]
    fn simple_runner_empty_pipeline() {
        let config = PipelineConfig::default();
        let runner = SimpleRunner::new(&config, 16000);
        assert!(runner.stats_snapshot().entries.is_empty());
        assert!(runner.stage_names().is_empty());
    }

    #[test]
    fn simple_runner_empty_pipeline_returns_true() {
        let config = PipelineConfig::default();
        let mut runner = SimpleRunner::new(&config, 16000);
        let mut samples = vec![0i16; 320];
        // Empty pipeline = always-on behavior
        assert!(runner.process_frame(&mut samples));
    }

    #[test]
    fn simple_runner_with_hpf() {
        let config = PipelineConfig {
            highpass: Some(crate::config::HighPassConfig {
                enabled: true,
                cutoff_hz: 85.0,
            }),
            ..PipelineConfig::default()
        };
        let mut runner = SimpleRunner::new(&config, 16000);

        let mut samples = vec![0i16; 320];
        let result = runner.process_frame(&mut samples);
        // HPF always returns true
        assert!(result);

        let snapshot = runner.stats_snapshot();
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(snapshot.entries[0].0, "highpass");
        assert_eq!(snapshot.entries[0].1, StatKey::EnergyRemoved);
        assert_eq!(snapshot.entries[1].0, "highpass");
        assert_eq!(snapshot.entries[1].1, StatKey::ProcessUs);
    }

    #[test]
    fn simple_runner_analyze_runs_at_interval() {
        let config = PipelineConfig {
            highpass: Some(crate::config::HighPassConfig {
                enabled: true,
                cutoff_hz: 85.0,
            }),
            analyze_interval: 3,
            ..PipelineConfig::default()
        };
        let mut runner = SimpleRunner::new(&config, 16000);
        let mut samples = vec![0i16; 320];

        for _ in 0..3 {
            runner.process_frame(&mut samples);
        }
        assert_eq!(runner.stage_names(), vec!["highpass"]);
    }

    #[test]
    fn pipeline_process_returns_bool() {
        let config = PipelineConfig {
            highpass: Some(crate::config::HighPassConfig {
                enabled: true,
                cutoff_hz: 85.0,
            }),
            energy_detector: Some(crate::config::EnergyDetectorConfig {
                attack_threshold: 500,
                sustain_threshold: 200,
            }),
            ..PipelineConfig::default()
        };
        let mut runner = SimpleRunner::new(&config, 16000);

        // Silence → detector returns false
        let mut silence = vec![0i16; 320];
        assert!(!runner.process_frame(&mut silence));

        // Loud alternating signal (survives HPF) → detector returns true
        let mut loud: Vec<i16> = (0..320).map(|i| if i % 2 == 0 { 5000 } else { -5000 }).collect();
        assert!(runner.process_frame(&mut loud));
    }

    #[test]
    fn pipeline_reset_propagates() {
        let config = PipelineConfig {
            energy_detector: Some(crate::config::EnergyDetectorConfig {
                attack_threshold: 500,
                sustain_threshold: 200,
            }),
            ..PipelineConfig::default()
        };
        let mut runner = SimpleRunner::new(&config, 16000);

        // Trigger to enter sustain phase
        let mut loud = vec![1000i16; 320];
        assert!(runner.process_frame(&mut loud));

        // Mid-level should sustain
        let mut mid = vec![300i16; 320];
        assert!(runner.process_frame(&mut mid));

        // Reset returns to attack phase
        runner.reset();

        // Mid-level no longer triggers (below attack=500)
        assert!(!runner.process_frame(&mut mid));
    }
}
