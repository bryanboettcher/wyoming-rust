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
// VAD (Voice Activity Detection) Trait
// ============================================================================

/// Abstraction over voice activity detection. Supports multiple modes:
/// - GPIO: Hardware trigger on a pin
/// - Energy: Software RMS threshold on audio frames
/// - AlwaysOn: Auto-trigger for testing
pub trait Vad {
    /// Check for voice activity. `audio_frame` is Some when mic is running.
    /// For GPIO mode, audio_frame is ignored. For Energy mode, it's required.
    fn poll(&mut self, audio_frame: Option<&[u8]>) -> bool;

    /// Whether mic must run during Idle (Energy/AlwaysOn: true, GPIO: false).
    fn needs_continuous_capture(&self) -> bool;

    /// Reset internal state on return to Idle.
    fn reset(&mut self);
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

// ============================================================================
// VAD Implementations
// ============================================================================

/// Always-on VAD for testing. Immediately triggers and requires continuous capture.
pub struct AlwaysOnVad;

impl AlwaysOnVad {
    pub fn new() -> Self {
        Self
    }
}

impl Vad for AlwaysOnVad {
    fn poll(&mut self, _audio_frame: Option<&[u8]>) -> bool {
        // Always return true — voice is "always detected"
        true
    }

    fn needs_continuous_capture(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        // No state to reset
    }
}

/// GPIO-based VAD. Reads a hardware pin for voice activity trigger.
/// Requires rppal on Raspberry Pi hardware.
pub struct GpioVad {
    #[cfg(target_os = "linux")]
    pin: rppal::gpio::InputPin,
    #[cfg(not(target_os = "linux"))]
    _phantom: std::marker::PhantomData<()>,
}

impl GpioVad {
    #[cfg(target_os = "linux")]
    pub fn new(pin_num: u32) -> Result<Self, AudioError> {
        use rppal::gpio::Gpio;

        let gpio = Gpio::new()
            .map_err(|e| AudioError::DeviceUnavailable(format!("GPIO init failed: {}", e)))?;

        let pin = gpio
            .get(pin_num as u8)
            .map_err(|e| AudioError::DeviceUnavailable(format!("GPIO pin {} unavailable: {}", pin_num, e)))?
            .into_input_pulldown();

        log::info!("GPIO VAD initialized on pin {}", pin_num);
        Ok(Self { pin })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(_pin_num: u32) -> Result<Self, AudioError> {
        Err(AudioError::DeviceUnavailable(
            "GPIO VAD requires Linux (rppal)".into(),
        ))
    }
}

impl Vad for GpioVad {
    fn poll(&mut self, _audio_frame: Option<&[u8]>) -> bool {
        #[cfg(target_os = "linux")]
        {
            use rppal::gpio::Level;
            self.pin.read() == Level::High
        }
        #[cfg(not(target_os = "linux"))]
        false
    }

    fn needs_continuous_capture(&self) -> bool {
        false
    }

    fn reset(&mut self) {
        // No state to reset
    }
}

/// Energy-based VAD. Computes RMS on PCM16LE frames and compares to threshold.
pub struct EnergyVad {
    threshold: u16,
}

impl EnergyVad {
    pub fn new(threshold: u16) -> Self {
        log::info!("Energy VAD initialized with threshold {}", threshold);
        Self { threshold }
    }

    /// Compute RMS (root mean square) of PCM16LE audio frame.
    /// Returns u16 in range 0-32768.
    fn compute_rms(frame: &[u8]) -> u16 {
        if frame.is_empty() {
            return 0;
        }

        // PCM16LE: every 2 bytes is one i16 sample
        let samples: Vec<i16> = frame
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        if samples.is_empty() {
            return 0;
        }

        // Compute sum of squares
        let sum_of_squares: i64 = samples.iter().map(|&s| (s as i64) * (s as i64)).sum();
        let mean_square = sum_of_squares / samples.len() as i64;
        let rms = (mean_square as f64).sqrt();

        // Clamp to u16 range
        rms.min(32768.0) as u16
    }
}

impl Vad for EnergyVad {
    fn poll(&mut self, audio_frame: Option<&[u8]>) -> bool {
        match audio_frame {
            Some(frame) => {
                let rms = Self::compute_rms(frame);
                rms >= self.threshold
            }
            None => {
                log::warn!("EnergyVad.poll() called without audio frame");
                false
            }
        }
    }

    fn needs_continuous_capture(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        // No state to reset (stateless threshold check)
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
    fn always_on_vad_always_triggers() {
        let mut vad = AlwaysOnVad::new();
        assert!(vad.poll(None));
        assert!(vad.poll(Some(&[0u8; 640])));
        assert!(vad.needs_continuous_capture());
    }

    #[test]
    fn energy_vad_silence_no_trigger() {
        let mut vad = EnergyVad::new(1000);
        let silence = vec![0u8; 640]; // All zeros = RMS 0
        assert!(!vad.poll(Some(&silence)));
        assert!(vad.needs_continuous_capture());
    }

    #[test]
    fn energy_vad_loud_frame_triggers() {
        let mut vad = EnergyVad::new(1000);
        // Create a loud frame: 320 samples at max amplitude
        let mut loud_frame = Vec::with_capacity(640);
        for _ in 0..320 {
            loud_frame.extend_from_slice(&10000i16.to_le_bytes());
        }
        assert!(vad.poll(Some(&loud_frame)));
    }

    #[test]
    fn energy_vad_threshold_boundary() {
        let mut vad = EnergyVad::new(5000);
        // Create frame right at threshold
        let mut frame = Vec::with_capacity(640);
        for _ in 0..320 {
            frame.extend_from_slice(&5000i16.to_le_bytes());
        }
        assert!(vad.poll(Some(&frame))); // RMS = 5000, threshold = 5000, should trigger
    }

    #[test]
    fn energy_vad_empty_frame_safe() {
        let mut vad = EnergyVad::new(1000);
        assert!(!vad.poll(Some(&[])));
    }

    #[test]
    fn energy_vad_no_frame_returns_false() {
        let mut vad = EnergyVad::new(1000);
        assert!(!vad.poll(None));
    }

    #[test]
    fn energy_vad_compute_rms() {
        // Silence
        let silence = vec![0u8; 640];
        assert_eq!(EnergyVad::compute_rms(&silence), 0);

        // Single sample at 1000
        let frame = 1000i16.to_le_bytes();
        assert_eq!(EnergyVad::compute_rms(&frame), 1000);

        // Empty frame
        assert_eq!(EnergyVad::compute_rms(&[]), 0);
    }
}
