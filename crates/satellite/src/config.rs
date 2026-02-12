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
    pub gpio: GpioConfig,
    #[serde(default)]
    pub led: LedConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SatelliteConfig {
    pub name: String,
    #[serde(default)]
    pub area: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
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

#[derive(Debug, Clone, Deserialize)]
pub struct GpioConfig {
    /// GPIO pin number for VAD Schmitt trigger input. None = auto-trigger mode.
    #[serde(default)]
    pub vad_pin: Option<u32>,

    /// How long after GPIO goes low before we stop streaming (ms).
    #[serde(default = "default_silence_timeout_ms")]
    pub silence_timeout_ms: u64,

    /// If true, automatically trigger (no GPIO hardware needed).
    #[serde(default)]
    pub auto_trigger: bool,
}

impl Default for GpioConfig {
    fn default() -> Self {
        Self {
            vad_pin: None,
            silence_timeout_ms: default_silence_timeout_ms(),
            auto_trigger: true,
        }
    }
}

fn default_silence_timeout_ms() -> u64 {
    2500
}

#[derive(Debug, Clone, Deserialize)]
pub struct LedConfig {
    #[serde(default)]
    pub pin: Option<u32>,

    /// "spi", "pwm", or "none".
    #[serde(default = "default_led_method")]
    pub method: String,
}

impl Default for LedConfig {
    fn default() -> Self {
        Self {
            pin: None,
            method: default_led_method(),
        }
    }
}

fn default_led_method() -> String {
    "none".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
    fn parse_hardware_config() {
        let toml = r#"
[satellite]
name = "living-room"
area = "Living Room"

[server]
host = "homeassistant.local"

[audio]
device = "hw:0,0"

[gpio]
vad_pin = 17
silence_timeout_ms = 2500

[led]
pin = 10
method = "spi"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.gpio.vad_pin, Some(17));
        assert_eq!(config.led.method, "spi");
        assert_eq!(config.server.port, 10700); // default
    }

    #[test]
    fn defaults_for_gpio_and_led() {
        let toml = r#"
[satellite]
name = "test"

[server]
host = "localhost"

[audio]
wav_input = "test.wav"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.gpio.auto_trigger);
        assert_eq!(config.led.method, "none");
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
}
