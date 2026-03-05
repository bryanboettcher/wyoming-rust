use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub satellite: SatelliteConfig,
    pub server: ServerConfig,
    pub audio: AudioConfig,
    #[serde(default)]
    pub feedback: Vec<FeedbackProviderConfig>,
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SatelliteConfig {
    pub name: String,
    #[serde(default)]
    pub area: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// "listen" (satellite is TCP server, HA connects in) or "connect" (satellite connects out).
    #[serde(default = "default_mode")]
    pub mode: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_mode() -> String {
    "listen".to_string()
}

fn default_port() -> u16 {
    10700
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioConfig {
    /// ALSA device name (e.g. "hw:0,0"). Mutually exclusive with wav_input.
    #[serde(default)]
    pub device: Option<String>,

    /// WAV file to read as mic input (testing mode).
    #[serde(default)]
    pub wav_input: Option<String>,

    /// ALSA device name for playback (e.g. "plughw:0,0"). Optional.
    #[serde(default)]
    pub playback_device: Option<String>,

    /// WAV file to write TTS output to (testing mode).
    #[serde(default)]
    pub wav_output: Option<String>,

    #[serde(default = "default_rate")]
    pub rate: u32,

    #[serde(default = "default_width")]
    pub width: u16,

    #[serde(default = "default_channels")]
    pub channels: u16,

    /// Milliseconds per audio frame (default 20ms = 320 samples at 16kHz).
    #[serde(default = "default_chunk_ms")]
    pub chunk_ms: u32,
}

fn default_rate() -> u32 {
    16000
}
fn default_width() -> u16 {
    2
}
fn default_channels() -> u16 {
    1
}
fn default_chunk_ms() -> u32 {
    20
}

impl AudioConfig {
    /// Bytes per audio frame based on rate, width, channels, and chunk_ms.
    pub fn frame_size(&self) -> usize {
        let samples_per_frame = (self.rate * self.chunk_ms) / 1000;
        samples_per_frame as usize * self.width as usize * self.channels as usize
    }

    pub fn audio_format(&self) -> wyoming::audio::AudioFormat {
        wyoming::audio::AudioFormat {
            rate: self.rate,
            width: self.width,
            channels: self.channels,
        }
    }
}

fn default_silence_timeout_ms() -> u64 {
    2500
}

fn default_buffer_seconds() -> f64 {
    1.0
}

// ============================================================================
// Audio Processing Pipeline Configuration
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct PipelineConfig {
    #[serde(default = "default_silence_timeout_ms")]
    pub silence_timeout_ms: u64,

    #[serde(default = "default_buffer_seconds")]
    pub buffer_seconds: f64,

    #[serde(default)]
    pub highpass: Option<HighPassConfig>,

    #[serde(default)]
    pub energy_detector: Option<EnergyDetectorConfig>,

    #[serde(default)]
    pub auto_energy: Option<AutoEnergyConfig>,

    #[serde(default)]
    pub bandpass: Option<BandpassConfig>,

    #[serde(default)]
    pub zcr: Option<ZcrConfig>,

    #[serde(default)]
    pub mfcc: Option<MfccConfig>,

    #[serde(default)]
    pub gate: Option<GateConfig>,

    #[serde(default)]
    pub agc: Option<AgcConfig>,

    #[serde(default = "default_analyze_interval")]
    pub analyze_interval: u64,

    /// Maximum duration (seconds) for a single streaming session. After this,
    /// the satellite forces a silence timeout even if the detector is still
    /// triggering. Prevents runaway streaming from stuck detection. 0 = no limit.
    #[serde(default = "default_max_stream_seconds")]
    pub max_stream_seconds: u64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            silence_timeout_ms: default_silence_timeout_ms(),
            buffer_seconds: default_buffer_seconds(),
            highpass: None,
            energy_detector: None,
            auto_energy: None,
            bandpass: None,
            zcr: None,
            mfcc: None,
            gate: None,
            agc: None,
            analyze_interval: default_analyze_interval(),
            max_stream_seconds: default_max_stream_seconds(),
        }
    }
}

fn default_max_stream_seconds() -> u64 {
    300 // 5 minutes
}

fn default_analyze_interval() -> u64 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct HighPassConfig {
    #[serde(default = "default_hpf_enabled")]
    pub enabled: bool,
    #[serde(default = "default_hpf_cutoff")]
    pub cutoff_hz: f32,
}

fn default_hpf_enabled() -> bool {
    true
}

fn default_hpf_cutoff() -> f32 {
    85.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnergyDetectorConfig {
    pub attack_threshold: u16,
    #[serde(default)]
    pub sustain_threshold: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutoEnergyConfig {
    #[serde(default = "default_attack_ratio")]
    pub attack_ratio: f32,
    #[serde(default = "default_sustain_ratio")]
    pub sustain_ratio: f32,
    /// Require N consecutive triggered frames before signaling true. 0 = disabled.
    #[serde(default)]
    pub hold_frames: u16,
}

fn default_attack_ratio() -> f32 {
    3.0
}

fn default_sustain_ratio() -> f32 {
    1.5
}

#[derive(Debug, Clone, Deserialize)]
pub struct ZcrConfig {
    #[serde(default = "default_zcr_enabled")]
    pub enabled: bool,
}

fn default_zcr_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct BandpassConfig {
    #[serde(default = "default_bandpass_enabled")]
    pub enabled: bool,
    #[serde(default = "default_bandpass_low")]
    pub low_hz: f32,
    #[serde(default = "default_bandpass_high")]
    pub high_hz: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MfccConfig {
    #[serde(default = "default_mfcc_enabled")]
    pub enabled: bool,
    #[serde(default = "default_n_mfcc")]
    pub n_mfcc: usize,
    #[serde(default = "default_n_mels")]
    pub n_mels: usize,
    #[serde(default = "default_n_fft")]
    pub n_fft: usize,
}

fn default_mfcc_enabled() -> bool {
    true
}
fn default_n_mfcc() -> usize {
    13
}
fn default_n_mels() -> usize {
    26
}
fn default_n_fft() -> usize {
    512
}

fn default_bandpass_enabled() -> bool {
    true
}

fn default_bandpass_low() -> f32 {
    300.0
}

fn default_bandpass_high() -> f32 {
    3000.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct GateConfig {
    #[serde(default = "default_gate_enabled")]
    pub enabled: bool,
    /// RMS threshold to open gate. 0 = auto (analyzer sets from noise floor).
    #[serde(default)]
    pub threshold: u16,
    #[serde(default = "default_gate_attack_ms")]
    pub attack_ms: f32,
    #[serde(default = "default_gate_release_ms")]
    pub release_ms: f32,
}

fn default_gate_enabled() -> bool {
    true
}

fn default_gate_attack_ms() -> f32 {
    1.0
}

fn default_gate_release_ms() -> f32 {
    50.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgcConfig {
    #[serde(default = "default_agc_enabled")]
    pub enabled: bool,
    #[serde(default = "default_agc_target_rms")]
    pub target_rms: f32,
    #[serde(default = "default_agc_max_gain")]
    pub max_gain: f32,
    #[serde(default = "default_agc_min_gain")]
    pub min_gain: f32,
}

fn default_agc_enabled() -> bool {
    true
}

fn default_agc_target_rms() -> f32 {
    3000.0
}

fn default_agc_max_gain() -> f32 {
    10.0
}

fn default_agc_min_gain() -> f32 {
    0.1
}

// ============================================================================
// Diagnostics / Health-Check Configuration
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsConfig {
    #[serde(default = "default_diagnostics_enabled")]
    pub enabled: bool,
    #[serde(default = "default_diagnostics_port")]
    pub port: u16,
    #[serde(default = "default_diagnostics_bind")]
    pub bind: String,
}

fn default_diagnostics_enabled() -> bool {
    true
}

fn default_diagnostics_port() -> u16 {
    8585
}

fn default_diagnostics_bind() -> String {
    "0.0.0.0".to_string()
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: default_diagnostics_enabled(),
            port: default_diagnostics_port(),
            bind: default_diagnostics_bind(),
        }
    }
}

// ============================================================================
// Feedback Provider Configuration
// ============================================================================

/// Per-state effect configuration container. Reused across all provider types.
/// Each field corresponds to a `FeedbackState` variant. `None` = noop for that state.
#[derive(Debug, Clone, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct StateEffects<T> {
    #[serde(default)]
    pub idle: Option<T>,
    #[serde(default)]
    pub listening: Option<T>,
    #[serde(default)]
    pub detected: Option<T>,
    #[serde(default)]
    pub processing: Option<T>,
    #[serde(default)]
    pub speaking: Option<T>,
    #[serde(default)]
    pub error: Option<T>,
}

// Manual Default — avoids requiring T: Default
impl<T> Default for StateEffects<T> {
    fn default() -> Self {
        Self {
            idle: None,
            listening: None,
            detected: None,
            processing: None,
            speaking: None,
            error: None,
        }
    }
}

impl<T> StateEffects<T> {
    pub fn get(&self, state: crate::state::FeedbackState) -> Option<&T> {
        use crate::state::FeedbackState;
        match state {
            FeedbackState::Idle => self.idle.as_ref(),
            FeedbackState::Listening => self.listening.as_ref(),
            FeedbackState::Detected => self.detected.as_ref(),
            FeedbackState::Processing => self.processing.as_ref(),
            FeedbackState::Speaking => self.speaking.as_ref(),
            FeedbackState::Error => self.error.as_ref(),
        }
    }
}

/// Feedback provider configuration. Discriminated by the `method` field.
///
/// Multiple providers can be configured using TOML's `[[feedback]]` array.
/// If none are specified, a logging-only provider is used by default.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "method")]
pub enum FeedbackProviderConfig {
    /// Logs state transitions via the `log` crate. No per-state config needed.
    #[serde(rename = "log")]
    Log {},

    /// Prints per-state templates to stdout or stderr.
    #[serde(rename = "console")]
    Console {
        #[serde(default = "default_console_output")]
        output: String,
        #[serde(default)]
        states: StateEffects<ConsoleEffect>,
    },

    /// Software PWM on a single GPIO pin. Covers LEDs (>1kHz) and buzzers (audible).
    #[serde(rename = "pwm")]
    Pwm {
        pin: u32,
        #[serde(default)]
        states: StateEffects<PwmEffect>,
    },

    /// WS2812/NeoPixel addressable RGB strip via SPI.
    #[serde(rename = "neopixel")]
    Neopixel {
        pin: u32,
        led_count: u32,
        #[serde(default)]
        spi_device: Option<String>,
        #[serde(default)]
        states: StateEffects<NeopixelEffect>,
    },
}

fn default_console_output() -> String {
    "stdout".to_string()
}

// ── Per-provider effect configs ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleEffect {
    pub template: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PwmEffect {
    #[serde(default = "default_pwm_effect")]
    pub effect: PwmEffectType,
    #[serde(default = "default_pwm_frequency")]
    pub frequency: f64,
    #[serde(default = "default_pwm_duty")]
    pub duty: f64,
    #[serde(default)]
    pub period_ms: Option<u64>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub pulse_ms: Option<u64>,
}

fn default_pwm_effect() -> PwmEffectType {
    PwmEffectType::Solid
}
fn default_pwm_frequency() -> f64 {
    1000.0
}
fn default_pwm_duty() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PwmEffectType {
    Off,
    Solid,
    Breathe,
    Blink,
    Pulse,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NeopixelEffect {
    #[serde(default = "default_neopixel_effect")]
    pub effect: NeopixelEffectType,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub period_ms: Option<u64>,
    #[serde(default)]
    pub brightness: Option<f64>,
}

fn default_neopixel_effect() -> NeopixelEffectType {
    NeopixelEffectType::Solid
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NeopixelEffectType {
    Off,
    Solid,
    Breathe,
    Rainbow,
    Blink,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.server.mode != "listen" && self.server.mode != "connect" {
            return Err(ConfigError::Invalid(format!(
                "server.mode must be 'listen' or 'connect', got '{}'",
                self.server.mode
            )));
        }
        if self.audio.device.is_none() && self.audio.wav_input.is_none() {
            return Err(ConfigError::Invalid(
                "audio config must specify either 'device' or 'wav_input'".into(),
            ));
        }
        if self.audio.device.is_some() && self.audio.wav_input.is_some() {
            return Err(ConfigError::Invalid(
                "audio config cannot specify both 'device' and 'wav_input'".into(),
            ));
        }
        let buffer_seconds = self.pipeline.buffer_seconds;
        if buffer_seconds.is_nan() || buffer_seconds.is_infinite() {
            return Err(ConfigError::Invalid(
                "pipeline.buffer_seconds must be a finite number".into(),
            ));
        }
        if buffer_seconds < 0.0 {
            return Err(ConfigError::Invalid(
                "pipeline.buffer_seconds must not be negative".into(),
            ));
        }
        if buffer_seconds > 10.0 {
            return Err(ConfigError::Invalid(format!(
                "pipeline.buffer_seconds must be <= 10.0, got {}",
                buffer_seconds
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_test_mode_config() {
        let toml = r#"
[satellite]
name = "test-satellite"
area = "Office"

[server]
host = "localhost"
port = 10700

[audio]
wav_input = "test.wav"
rate = 16000
width = 2
channels = 1
chunk_ms = 20
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.satellite.name, "test-satellite");
        assert_eq!(config.server.host, "localhost");
        assert_eq!(config.server.port, 10700);
        assert_eq!(config.audio.wav_input.as_deref(), Some("test.wav"));
        assert!(config.audio.device.is_none());
        assert_eq!(config.audio.frame_size(), 640); // 16000 * 20 / 1000 * 2 * 1
    }

    #[test]
    fn defaults_for_vad_and_feedback() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.pipeline.silence_timeout_ms, 2500);
        assert!(config.feedback.is_empty());
    }

    #[test]
    fn multiple_feedback_providers() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "neopixel"
pin = 10
led_count = 3

[[feedback]]
method = "pwm"
pin = 25

[[feedback]]
method = "log"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.feedback.len(), 3);
        assert!(matches!(&config.feedback[0], FeedbackProviderConfig::Neopixel { pin: 10, led_count: 3, .. }));
        assert!(matches!(&config.feedback[1], FeedbackProviderConfig::Pwm { pin: 25, .. }));
        assert!(matches!(&config.feedback[2], FeedbackProviderConfig::Log {}));
    }

    #[test]
    fn console_provider_with_states() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "console"
output = "stderr"

[feedback.states]
idle = { template = "Ready" }
listening = { template = "Listening..." }
detected = { template = "Wake word!" }
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.feedback.len(), 1);
        match &config.feedback[0] {
            FeedbackProviderConfig::Console { output, states } => {
                assert_eq!(output, "stderr");
                assert_eq!(states.idle.as_ref().unwrap().template, "Ready");
                assert_eq!(states.listening.as_ref().unwrap().template, "Listening...");
                assert_eq!(states.detected.as_ref().unwrap().template, "Wake word!");
                assert!(states.processing.is_none());
                assert!(states.speaking.is_none());
                assert!(states.error.is_none());
            }
            other => panic!("expected Console, got {:?}", other),
        }
    }

    #[test]
    fn pwm_provider_with_effects() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "pwm"
pin = 24

[feedback.states.listening]
effect = "breathe"
frequency = 1000.0
period_ms = 2000

[feedback.states.detected]
effect = "solid"
frequency = 880.0
duty = 0.5
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.feedback[0] {
            FeedbackProviderConfig::Pwm { pin, states } => {
                assert_eq!(*pin, 24);
                let listening = states.listening.as_ref().unwrap();
                assert_eq!(listening.effect, PwmEffectType::Breathe);
                assert_eq!(listening.frequency, 1000.0);
                assert_eq!(listening.period_ms, Some(2000));
                let detected = states.detected.as_ref().unwrap();
                assert_eq!(detected.effect, PwmEffectType::Solid);
                assert_eq!(detected.frequency, 880.0);
                assert_eq!(detected.duty, 0.5);
            }
            other => panic!("expected Pwm, got {:?}", other),
        }
    }

    #[test]
    fn neopixel_provider_with_hex_colors() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "neopixel"
pin = 8
led_count = 3

[feedback.states.idle]
effect = "solid"
color = 0x000800
brightness = 0.1

[feedback.states.detected]
effect = "rainbow"
period_ms = 1000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.feedback[0] {
            FeedbackProviderConfig::Neopixel { pin, led_count, states, .. } => {
                assert_eq!(*pin, 8);
                assert_eq!(*led_count, 3);
                let idle = states.idle.as_ref().unwrap();
                assert_eq!(idle.effect, NeopixelEffectType::Solid);
                assert_eq!(idle.color, Some(0x000800));
                assert_eq!(idle.brightness, Some(0.1));
                let detected = states.detected.as_ref().unwrap();
                assert_eq!(detected.effect, NeopixelEffectType::Rainbow);
                assert_eq!(detected.period_ms, Some(1000));
                assert!(detected.color.is_none());
            }
            other => panic!("expected Neopixel, got {:?}", other),
        }
    }

    #[test]
    fn unknown_feedback_method_rejected_at_parse() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "unknown_thing"
"#;
        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn two_neopixel_strips() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "neopixel"
pin = 8
led_count = 3

[[feedback]]
method = "neopixel"
pin = 10
led_count = 12
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.feedback.len(), 2);
        match (&config.feedback[0], &config.feedback[1]) {
            (
                FeedbackProviderConfig::Neopixel { pin: p1, led_count: l1, .. },
                FeedbackProviderConfig::Neopixel { pin: p2, led_count: l2, .. },
            ) => {
                assert_eq!(*p1, 8);
                assert_eq!(*l1, 3);
                assert_eq!(*p2, 10);
                assert_eq!(*l2, 12);
            }
            other => panic!("expected two Neopixels, got {:?}", other),
        }
    }

    #[test]
    fn state_effects_get_maps_correctly() {
        use crate::state::FeedbackState;

        let effects = StateEffects {
            idle: Some(ConsoleEffect { template: "idle".into() }),
            listening: Some(ConsoleEffect { template: "listening".into() }),
            detected: None,
            processing: None,
            speaking: None,
            error: Some(ConsoleEffect { template: "error".into() }),
        };

        assert_eq!(effects.get(FeedbackState::Idle).unwrap().template, "idle");
        assert_eq!(effects.get(FeedbackState::Listening).unwrap().template, "listening");
        assert!(effects.get(FeedbackState::Detected).is_none());
        assert!(effects.get(FeedbackState::Processing).is_none());
        assert!(effects.get(FeedbackState::Speaking).is_none());
        assert_eq!(effects.get(FeedbackState::Error).unwrap().template, "error");
    }

    #[test]
    fn pwm_effect_defaults() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[[feedback]]
method = "pwm"
pin = 24

[feedback.states.listening]
effect = "breathe"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.feedback[0] {
            FeedbackProviderConfig::Pwm { states, .. } => {
                let listening = states.listening.as_ref().unwrap();
                assert_eq!(listening.effect, PwmEffectType::Breathe);
                assert_eq!(listening.frequency, 1000.0); // default
                assert_eq!(listening.duty, 1.0); // default
                assert!(listening.period_ms.is_none());
            }
            other => panic!("expected Pwm, got {:?}", other),
        }
    }

    #[test]
    fn validates_audio_source_required() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
rate = 16000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validates_audio_source_exclusive() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
device = "hw:0,0"
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn frame_size_calculation() {
        let config = AudioConfig {
            device: None,
            playback_device: None,
            wav_input: Some("test.wav".into()),
            wav_output: None,
            rate: 16000,
            width: 2,
            channels: 1,
            chunk_ms: 20,
        };
        // 16000 Hz * 20ms / 1000 = 320 samples * 2 bytes * 1 channel = 640 bytes
        assert_eq!(config.frame_size(), 640);
    }

    #[test]
    fn server_mode_defaults_to_listen() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.server.mode, "listen");
    }

    #[test]
    fn server_mode_connect_accepted() {
        let toml = r#"
[satellite]
name = "test"

[server]
mode = "connect"
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        config.validate().unwrap();
        assert_eq!(config.server.mode, "connect");
    }

    #[test]
    fn server_mode_invalid_rejected() {
        let toml = r#"
[satellite]
name = "test"

[server]
mode = "push"
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn buffer_seconds_invalid_values_rejected() {
        fn make_config(buffer_seconds: f64) -> Config {
            Config {
                satellite: SatelliteConfig { name: "test".into(), area: None },
                server: ServerConfig { mode: "listen".into(), host: "localhost".into(), port: 10700 },
                audio: AudioConfig {
                    device: None,
                    wav_input: Some("test.wav".into()),
                    playback_device: None,
                    wav_output: None,
                    rate: 16000,
                    width: 2,
                    channels: 1,
                    chunk_ms: 20,
                },
                feedback: vec![],
                diagnostics: DiagnosticsConfig::default(),
                pipeline: PipelineConfig {
                    buffer_seconds,
                    ..PipelineConfig::default()
                },
            }
        }

        assert!(make_config(-1.0).validate().is_err(), "negative buffer_seconds should be rejected");
        assert!(make_config(10.1).validate().is_err(), "buffer_seconds > 10.0 should be rejected");
        assert!(make_config(10.0).validate().is_ok(), "buffer_seconds == 10.0 should be accepted");
        assert!(make_config(0.0).validate().is_ok(), "buffer_seconds == 0.0 should be accepted");
    }

    #[test]
    fn diagnostics_defaults_when_section_omitted() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.diagnostics.enabled);
        assert_eq!(config.diagnostics.port, 8585);
        assert_eq!(config.diagnostics.bind, "0.0.0.0");
    }

    #[test]
    fn diagnostics_explicit_config() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[diagnostics]
enabled = false
port = 9090
bind = "127.0.0.1"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.diagnostics.enabled);
        assert_eq!(config.diagnostics.port, 9090);
        assert_eq!(config.diagnostics.bind, "127.0.0.1");
    }

    #[test]
    fn diagnostics_partial_config() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[diagnostics]
port = 3000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.diagnostics.enabled); // default
        assert_eq!(config.diagnostics.port, 3000);
        assert_eq!(config.diagnostics.bind, "0.0.0.0"); // default
    }

    #[test]
    fn pipeline_config_with_auto_energy() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[pipeline]
silence_timeout_ms = 3000
buffer_seconds = 2.0

[pipeline.highpass]
cutoff_hz = 85.0

[pipeline.auto_energy]
attack_ratio = 4.0
sustain_ratio = 2.0
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.pipeline.silence_timeout_ms, 3000);
        assert_eq!(config.pipeline.buffer_seconds, 2.0);
        let hp = config.pipeline.highpass.unwrap();
        assert!(hp.enabled);
        assert_eq!(hp.cutoff_hz, 85.0);
        let ae = config.pipeline.auto_energy.unwrap();
        assert_eq!(ae.attack_ratio, 4.0);
        assert_eq!(ae.sustain_ratio, 2.0);
    }

    #[test]
    fn pipeline_config_with_energy_detector() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[pipeline.energy_detector]
attack_threshold = 500
sustain_threshold = 200
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ed = config.pipeline.energy_detector.unwrap();
        assert_eq!(ed.attack_threshold, 500);
        assert_eq!(ed.sustain_threshold, 200);
    }

    #[test]
    fn pipeline_defaults_to_always_on() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        // No pipeline section → default PipelineConfig with no stages = always-on
        assert!(config.pipeline.highpass.is_none());
        assert!(config.pipeline.energy_detector.is_none());
        assert!(config.pipeline.auto_energy.is_none());
        assert_eq!(config.pipeline.silence_timeout_ms, 2500);
        assert_eq!(config.pipeline.buffer_seconds, 1.0);
    }
}
