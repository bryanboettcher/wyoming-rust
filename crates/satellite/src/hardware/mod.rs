pub mod alsa;

use std::time::Duration;
use thiserror::Error;
use wyoming::audio::AudioFormat;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("WAV format error: {0}")]
    WavFormat(String),

    #[error("end of audio stream")]
    EndOfStream,

    #[error("audio device not available: {0}")]
    DeviceUnavailable(String),

    #[error("ALSA error: {0}")]
    Alsa(String),
}

// ============================================================================
// Audio Source Trait
// ============================================================================

/// Abstraction over mic input. Swap ALSA for WAV file in testing.
pub trait AudioSource {
    /// Read one frame of PCM audio. Blocks for ~chunk_ms duration.
    fn read_frame(&mut self) -> Result<Vec<u8>, AudioError>;

    /// Prepare the audio source for capture.
    fn start(&mut self) -> Result<(), AudioError>;

    /// Stop capturing audio.
    fn stop(&mut self) -> Result<(), AudioError>;
}

// ============================================================================
// Audio Sink Trait
// ============================================================================

/// Abstraction over speaker output.
pub trait AudioSink {
    /// Write one frame of PCM audio to the output.
    fn write_frame(&mut self, pcm: &[u8]) -> Result<(), AudioError>;

    /// Prepare the audio sink for playback with the given format.
    /// The format comes from the server's `audio-start` event (TTS rate/channels).
    fn start(&mut self, format: AudioFormat) -> Result<(), AudioError>;

    /// Stop playback.
    fn stop(&mut self) -> Result<(), AudioError>;
}

// ============================================================================
// GPIO Input Trait
// ============================================================================

/// Abstraction over a GPIO pin used for voice activity detection.
pub trait GpioInput {
    /// Check if the trigger pin is currently high.
    fn is_high(&self) -> bool;

    /// Block until the pin goes high or timeout expires.
    /// Returns true if the pin went high, false on timeout.
    fn wait_for_high(&self, timeout: Duration) -> bool;
}

// ============================================================================
// Test Doubles
// ============================================================================

/// Reads a WAV file as if it were a microphone, returning frames at the
/// configured chunk size. After the file is exhausted, returns EndOfStream.
pub struct WavFileSource {
    samples: Vec<u8>,
    position: usize,
    frame_size: usize,
    frame_duration: Duration,
    running: bool,
}

impl WavFileSource {
    pub fn open(path: &str, frame_size: usize, chunk_ms: u32) -> Result<Self, AudioError> {
        let reader = hound::WavReader::open(path).map_err(|e| {
            AudioError::WavFormat(format!("failed to open {}: {}", path, e))
        })?;

        let spec = reader.spec();
        if spec.bits_per_sample != 16 || spec.channels != 1 {
            log::warn!(
                "WAV file has {}ch {}bit — Wyoming expects mono 16-bit. Proceeding anyway.",
                spec.channels,
                spec.bits_per_sample
            );
        }

        // Read all samples as raw bytes
        let samples: Vec<i16> = reader
            .into_samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AudioError::WavFormat(format!("failed to read samples: {}", e)))?;

        // Convert i16 samples to little-endian bytes (PCM16LE)
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in &samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        log::info!(
            "Loaded WAV: {} samples ({} bytes), {}Hz {}ch",
            samples.len(),
            bytes.len(),
            spec.sample_rate,
            spec.channels
        );

        Ok(Self {
            samples: bytes,
            position: 0,
            frame_size,
            frame_duration: Duration::from_millis(chunk_ms as u64),
            running: false,
        })
    }
}

impl AudioSource for WavFileSource {
    fn read_frame(&mut self) -> Result<Vec<u8>, AudioError> {
        if !self.running {
            return Err(AudioError::DeviceUnavailable("source not started".into()));
        }

        if self.position >= self.samples.len() {
            return Err(AudioError::EndOfStream);
        }

        let end = (self.position + self.frame_size).min(self.samples.len());
        let mut frame = self.samples[self.position..end].to_vec();

        // Pad with silence if we don't have a full frame
        if frame.len() < self.frame_size {
            frame.resize(self.frame_size, 0);
        }

        self.position += self.frame_size;

        // Simulate real-time mic timing
        std::thread::sleep(self.frame_duration);

        Ok(frame)
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.position = 0;
        self.running = true;
        log::debug!("WavFileSource: started");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.running = false;
        log::debug!("WavFileSource: stopped");
        Ok(())
    }
}

/// Discards all audio written to it.
pub struct NullSink;

impl AudioSink for NullSink {
    fn write_frame(&mut self, _pcm: &[u8]) -> Result<(), AudioError> {
        Ok(())
    }
    fn start(&mut self, _format: AudioFormat) -> Result<(), AudioError> {
        log::debug!("NullSink: started");
        Ok(())
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        log::debug!("NullSink: stopped");
        Ok(())
    }
}

/// Writes received TTS audio to a WAV file.
pub struct WavFileSink {
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    path: String,
    rate: u32,
    channels: u16,
}

impl WavFileSink {
    pub fn new(path: &str, rate: u32, channels: u16) -> Self {
        Self {
            writer: None,
            path: path.to_string(),
            rate,
            channels,
        }
    }

    fn ensure_writer(
        &mut self,
    ) -> Result<&mut hound::WavWriter<std::io::BufWriter<std::fs::File>>, AudioError> {
        if self.writer.is_none() {
            let spec = hound::WavSpec {
                channels: self.channels,
                sample_rate: self.rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            self.writer = Some(hound::WavWriter::create(&self.path, spec).map_err(|e| {
                AudioError::WavFormat(format!("failed to create {}: {}", self.path, e))
            })?);
            log::info!("WavFileSink: writing to {}", self.path);
        }
        Ok(self.writer.as_mut().unwrap())
    }
}

impl AudioSink for WavFileSink {
    fn write_frame(&mut self, pcm: &[u8]) -> Result<(), AudioError> {
        let writer = self.ensure_writer()?;
        // PCM16LE: every 2 bytes is one i16 sample
        for chunk in pcm.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            writer
                .write_sample(sample)
                .map_err(|e| AudioError::WavFormat(format!("write error: {}", e)))?;
        }
        Ok(())
    }

    fn start(&mut self, _format: AudioFormat) -> Result<(), AudioError> {
        log::debug!("WavFileSink: started");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(writer) = self.writer.take() {
            writer
                .finalize()
                .map_err(|e| AudioError::WavFormat(format!("finalize error: {}", e)))?;
            log::info!("WavFileSink: finalized {}", self.path);
        }
        Ok(())
    }
}

/// Auto-triggering GPIO for testing. Always fires GpioHigh immediately
/// when polled, and always reports low for `is_high()`. The silence
/// timeout in the main loop (tracked via `last_gpio_high` timestamp)
/// handles the streaming duration.
pub struct AutoGpio;

impl AutoGpio {
    pub fn new() -> Self {
        Self
    }
}

impl GpioInput for AutoGpio {
    fn is_high(&self) -> bool {
        // Always low — the main loop tracks the timeout from the initial trigger
        false
    }

    fn wait_for_high(&self, _timeout: Duration) -> bool {
        // Immediately trigger in test mode
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_accepts_all_frames() {
        let mut sink = NullSink;
        sink.start(AudioFormat::WYOMING_DEFAULT).unwrap();
        sink.write_frame(&[0u8; 640]).unwrap();
        sink.write_frame(&[0u8; 320]).unwrap();
        sink.stop().unwrap();
    }

    #[test]
    fn auto_gpio_fires_immediately() {
        let gpio = AutoGpio::new();
        assert!(gpio.wait_for_high(Duration::from_millis(100)));
    }

    #[test]
    fn auto_gpio_is_always_low() {
        let gpio = AutoGpio::new();
        assert!(!gpio.is_high());
    }
}
