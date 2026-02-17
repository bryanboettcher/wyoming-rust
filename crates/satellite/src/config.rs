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
    pub vad: VadConfig,
    #[serde(default)]
    pub feedback: Vec<FeedbackProviderConfig>,
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

/// Voice Activity Detection configuration. Discriminated by the `mode` field.
///
/// Three modes are supported:
/// - `always_on`: Auto-trigger mode for testing (default)
/// - `gpio`: Hardware button/trigger on a GPIO pin
/// - `energy`: Software RMS-based VAD on audio frames
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode")]
pub enum VadConfig {
    #[serde(rename = "always_on")]
    AlwaysOn {
        #[serde(default = "default_silence_timeout_ms")]
        silence_timeout_ms: u64,
    },

    #[serde(rename = "gpio")]
    Gpio {
        pin: u32,
        #[serde(default = "default_silence_timeout_ms")]
        silence_timeout_ms: u64,
    },

    #[serde(rename = "energy")]
    Energy {
        threshold: u16,
        #[serde(default = "default_silence_timeout_ms")]
        silence_timeout_ms: u64,
    },
}

impl Default for VadConfig {
    fn default() -> Self {
        VadConfig::AlwaysOn {
            silence_timeout_ms: default_silence_timeout_ms(),
        }
    }
}

impl VadConfig {
    /// Get the silence timeout in milliseconds for this VAD mode.
    pub fn silence_timeout_ms(&self) -> u64 {
        match self {
            VadConfig::AlwaysOn { silence_timeout_ms } => *silence_timeout_ms,
            VadConfig::Gpio { silence_timeout_ms, .. } => *silence_timeout_ms,
            VadConfig::Energy { silence_timeout_ms, .. } => *silence_timeout_ms,
        }
    }
}

fn default_silence_timeout_ms() -> u64 {
    2500
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
    fn parse_hardware_config_with_gpio_vad() {
        let toml = r#"
[satellite]
name = "living-room"
area = "Living Room"

[server]
host = "homeassistant.local"

[audio]
device = "hw:0,0"

[vad]
mode = "gpio"
pin = 17
silence_timeout_ms = 2500

[[feedback]]
method = "neopixel"
pin = 10
led_count = 3
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.vad {
            VadConfig::Gpio { pin, silence_timeout_ms } => {
                assert_eq!(*pin, 17);
                assert_eq!(*silence_timeout_ms, 2500);
            }
            other => panic!("expected Gpio VAD, got {:?}", other),
        }
        assert_eq!(config.feedback.len(), 1);
        match &config.feedback[0] {
            FeedbackProviderConfig::Neopixel { pin, led_count, .. } => {
                assert_eq!(*pin, 10);
                assert_eq!(*led_count, 3);
            }
            other => panic!("expected Neopixel, got {:?}", other),
        }
        assert_eq!(config.server.port, 10700); // default
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
        assert!(matches!(config.vad, VadConfig::AlwaysOn { .. }));
        assert_eq!(config.vad.silence_timeout_ms(), 2500);
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
    fn vad_mode_always_on() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[vad]
mode = "always_on"
silence_timeout_ms = 3000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.vad {
            VadConfig::AlwaysOn { silence_timeout_ms } => {
                assert_eq!(*silence_timeout_ms, 3000);
            }
            other => panic!("expected AlwaysOn VAD, got {:?}", other),
        }
        assert_eq!(config.vad.silence_timeout_ms(), 3000);
    }

    #[test]
    fn vad_mode_gpio() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[vad]
mode = "gpio"
pin = 23
silence_timeout_ms = 1500
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.vad {
            VadConfig::Gpio { pin, silence_timeout_ms } => {
                assert_eq!(*pin, 23);
                assert_eq!(*silence_timeout_ms, 1500);
            }
            other => panic!("expected Gpio VAD, got {:?}", other),
        }
        assert_eq!(config.vad.silence_timeout_ms(), 1500);
    }

    #[test]
    fn vad_mode_energy() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[vad]
mode = "energy"
threshold = 1500
silence_timeout_ms = 2000
"#;
        let config: Config = toml::from_str(toml).unwrap();
        match &config.vad {
            VadConfig::Energy { threshold, silence_timeout_ms } => {
                assert_eq!(*threshold, 1500);
                assert_eq!(*silence_timeout_ms, 2000);
            }
            other => panic!("expected Energy VAD, got {:?}", other),
        }
        assert_eq!(config.vad.silence_timeout_ms(), 2000);
    }

    #[test]
    fn vad_mode_gpio_with_default_timeout() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[vad]
mode = "gpio"
pin = 17
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.vad.silence_timeout_ms(), 2500); // default
    }

    #[test]
    fn vad_mode_unknown_rejected() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"

[vad]
mode = "invalid_mode"
"#;
        let result: Result<Config, _> = toml::from_str(toml);
        assert!(result.is_err());
    }
}
