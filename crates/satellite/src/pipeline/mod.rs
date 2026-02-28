pub mod hpf;

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
        }
    }
}

pub trait Stage: Send {
    fn process(&mut self, samples: &mut [i16]);
    fn analyze(self) -> Self where Self: Sized;
    fn stats(&self) -> &[(StatKey, f64)];
    fn name(&self) -> &'static str;
}

pub enum StageKind {
    HighPass(hpf::HighPassFilter),
}

impl StageKind {
    pub fn process(&mut self, samples: &mut [i16]) {
        match self {
            Self::HighPass(s) => s.process(samples),
        }
    }

    pub fn analyze(self) -> Self {
        match self {
            Self::HighPass(s) => Self::HighPass(s.analyze()),
        }
    }

    pub fn stats(&self) -> &[(StatKey, f64)] {
        match self {
            Self::HighPass(s) => s.stats(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::HighPass(s) => s.name(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatsSnapshot {
    pub entries: Vec<(&'static str, StatKey, f64)>,
}

pub trait Runner: Send {
    fn process_frame(&mut self, samples: &mut [i16]);
    fn is_active(&self) -> bool;
    fn stats_snapshot(&self) -> StatsSnapshot;
}

pub struct SimpleRunner {
    stages: Vec<StageKind>,
    frame_count: u64,
    analyze_interval: u64,
    stage_timings_us: Vec<u32>,
    last_analyze_us: u32,
}

impl SimpleRunner {
    pub fn new(config: &PipelineConfig, sample_rate: u32) -> Self {
        let mut stages = Vec::new();

        if let Some(ref hpf_config) = config.highpass {
            if hpf_config.enabled {
                stages.push(StageKind::HighPass(
                    hpf::HighPassFilter::new(hpf_config.cutoff_hz, sample_rate),
                ));
            }
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
        }
    }
}

impl Runner for SimpleRunner {
    fn process_frame(&mut self, samples: &mut [i16]) {
        for (i, stage) in self.stages.iter_mut().enumerate() {
            let t = Instant::now();
            stage.process(samples);
            self.stage_timings_us[i] = t.elapsed().as_micros() as u32;
        }
        self.frame_count += 1;
        if self.frame_count % self.analyze_interval == 0 {
            let t = Instant::now();
            self.stages = self.stages.drain(..).map(|s| s.analyze()).collect();
            self.last_analyze_us = t.elapsed().as_micros() as u32;
        }
    }

    fn is_active(&self) -> bool {
        !self.stages.is_empty()
    }

    fn stats_snapshot(&self) -> StatsSnapshot {
        let mut entries = Vec::new();
        for (i, stage) in self.stages.iter().enumerate() {
            for &(key, val) in stage.stats() {
                entries.push((stage.name(), key, val));
            }
            if let Some(&us) = self.stage_timings_us.get(i) {
                entries.push((stage.name(), StatKey::ProcessUs, us as f64));
            }
        }
        if self.last_analyze_us > 0 {
            entries.push(("runner", StatKey::AnalyzeUs, self.last_analyze_us as f64));
        }
        StatsSnapshot { entries }
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
        assert!(!runner.is_active());
        assert!(runner.stats_snapshot().entries.is_empty());
    }

    #[test]
    fn simple_runner_with_hpf() {
        let config = PipelineConfig {
            enabled: true,
            highpass: Some(crate::config::HighPassConfig {
                enabled: true,
                cutoff_hz: 85.0,
            }),
            gate: None,
            agc: None,
            analyze_interval: 50,
        };
        let mut runner = SimpleRunner::new(&config, 16000);
        assert!(runner.is_active());

        let mut samples = vec![0i16; 320];
        runner.process_frame(&mut samples);

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
            enabled: true,
            highpass: Some(crate::config::HighPassConfig {
                enabled: true,
                cutoff_hz: 85.0,
            }),
            gate: None,
            agc: None,
            analyze_interval: 3,
        };
        let mut runner = SimpleRunner::new(&config, 16000);
        let mut samples = vec![0i16; 320];

        for _ in 0..3 {
            runner.process_frame(&mut samples);
        }
        assert!(runner.is_active());
    }
}
