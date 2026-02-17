use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};
use wyoming::audio::AudioFormat;

use super::{AudioError, AudioSink, AudioSource};

/// Map an alsa::Error to our AudioError.
fn alsa_err(e: alsa::Error) -> AudioError {
    AudioError::Alsa(e.to_string())
}

// ============================================================================
// AlsaSource — microphone capture
// ============================================================================

/// ALSA PCM capture source for real microphone input.
///
/// Opens the ALSA device on `start()` and closes it on `stop()`.
/// Each `read_frame()` blocks for approximately one period (~20ms at 16kHz).
pub struct AlsaSource {
    device: String,
    pcm: Option<PCM>,
    frame_size: usize,
    period_frames: u32,
    rate: u32,
    channels: u16,
}

impl AlsaSource {
    pub fn new(device: &str, rate: u32, channels: u16, frame_size: usize) -> Self {
        // period_frames = samples per period (frame_size bytes / (width * channels))
        // For 16-bit mono: 640 bytes / 2 = 320 frames
        let bytes_per_frame = 2 * channels as usize; // 16-bit = 2 bytes per sample per channel
        let period_frames = (frame_size / bytes_per_frame) as u32;

        Self {
            device: device.to_string(),
            pcm: None,
            frame_size,
            period_frames,
            rate,
            channels,
        }
    }

    fn open_device(&mut self) -> Result<(), AudioError> {
        log::info!(
            "Opening ALSA capture device '{}' ({}Hz, {}ch, period={})",
            self.device,
            self.rate,
            self.channels,
            self.period_frames
        );

        let pcm = PCM::new(&self.device, Direction::Capture, false).map_err(|e| {
            AudioError::DeviceUnavailable(format!("{}: {}", self.device, e))
        })?;

        // Configure hardware parameters
        {
            let hwp = HwParams::any(&pcm).map_err(alsa_err)?;
            hwp.set_format(Format::s16()).map_err(alsa_err)?;
            hwp.set_access(Access::RWInterleaved).map_err(alsa_err)?;
            hwp.set_channels(self.channels as u32).map_err(alsa_err)?;
            hwp.set_rate(self.rate, ValueOr::Nearest).map_err(alsa_err)?;
            // Frames is c_long: i32 on 32-bit ARM, i64 on 64-bit x86_64
            hwp.set_period_size(self.period_frames as _, ValueOr::Nearest)
                .map_err(alsa_err)?;
            // Buffer = 5 periods gives ~100ms of margin
            hwp.set_buffer_size((self.period_frames * 5) as _)
                .map_err(alsa_err)?;
            pcm.hw_params(&hwp).map_err(alsa_err)?;

            let actual_rate = hwp.get_rate().map_err(alsa_err)?;
            let actual_period = hwp.get_period_size().map_err(alsa_err)?;
            let actual_buffer = hwp.get_buffer_size().map_err(alsa_err)?;
            log::info!(
                "ALSA capture configured: rate={}Hz, period={} frames, buffer={} frames",
                actual_rate,
                actual_period,
                actual_buffer
            );
        }

        pcm.prepare().map_err(alsa_err)?;
        self.pcm = Some(pcm);
        Ok(())
    }
}

impl AudioSource for AlsaSource {
    fn read_frame(&mut self) -> Result<Vec<u8>, AudioError> {
        let pcm = self
            .pcm
            .as_ref()
            .ok_or_else(|| AudioError::DeviceUnavailable("capture device not started".into()))?;

        let samples_per_frame = self.period_frames as usize * self.channels as usize;
        let mut buf = vec![0i16; samples_per_frame];

        let io = pcm.io_i16().map_err(alsa_err)?;

        match io.readi(&mut buf) {
            Ok(frames_read) => {
                // Convert i16 samples to PCM16LE bytes
                let actual_samples = frames_read * self.channels as usize;
                let mut bytes = Vec::with_capacity(actual_samples * 2);
                for &sample in &buf[..actual_samples] {
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
                // Pad to frame_size if short
                if bytes.len() < self.frame_size {
                    bytes.resize(self.frame_size, 0);
                }
                Ok(bytes)
            }
            Err(e) => {
                // Try to recover from overrun (EPIPE)
                if pcm.try_recover(e, true).is_ok() {
                    log::warn!("ALSA capture: recovered from overrun");
                    // Retry once after recovery
                    match io.readi(&mut buf) {
                        Ok(frames_read) => {
                            let actual_samples = frames_read * self.channels as usize;
                            let mut bytes = Vec::with_capacity(actual_samples * 2);
                            for &sample in &buf[..actual_samples] {
                                bytes.extend_from_slice(&sample.to_le_bytes());
                            }
                            if bytes.len() < self.frame_size {
                                bytes.resize(self.frame_size, 0);
                            }
                            Ok(bytes)
                        }
                        Err(e2) => Err(alsa_err(e2)),
                    }
                } else {
                    Err(alsa_err(e))
                }
            }
        }
    }

    fn start(&mut self) -> Result<(), AudioError> {
        self.open_device()?;
        log::debug!("AlsaSource: started");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(pcm) = self.pcm.take() {
            let _ = pcm.drop();
        }
        log::debug!("AlsaSource: stopped");
        Ok(())
    }
}

// ============================================================================
// AlsaSink — speaker playback
// ============================================================================

/// ALSA PCM playback sink for real speaker output.
///
/// Opens and configures the ALSA device on `start()` using the provided
/// AudioFormat from the server's `audio-start` event. Reconfigures if
/// the format changes between sessions.
pub struct AlsaSink {
    device: String,
    pcm: Option<PCM>,
    current_format: Option<AudioFormat>,
}

impl AlsaSink {
    pub fn new(device: &str) -> Self {
        Self {
            device: device.to_string(),
            pcm: None,
            current_format: None,
        }
    }

    fn open_device(&mut self, format: AudioFormat) -> Result<(), AudioError> {
        // Close existing device if format changed
        if let Some(pcm) = self.pcm.take() {
            let _ = pcm.drain();
            let _ = pcm.drop();
        }

        // Compute period size from format: 20ms worth of frames
        let period_frames = (format.rate * 20) / 1000; // e.g., 22050*20/1000 = 441

        log::info!(
            "Opening ALSA playback device '{}' ({}Hz, {}ch, period={})",
            self.device,
            format.rate,
            format.channels,
            period_frames
        );

        let pcm = PCM::new(&self.device, Direction::Playback, false).map_err(|e| {
            AudioError::DeviceUnavailable(format!("{}: {}", self.device, e))
        })?;

        // Configure hardware parameters
        {
            let hwp = HwParams::any(&pcm).map_err(alsa_err)?;
            hwp.set_format(Format::s16()).map_err(alsa_err)?;
            hwp.set_access(Access::RWInterleaved).map_err(alsa_err)?;
            hwp.set_channels(format.channels as u32).map_err(alsa_err)?;
            hwp.set_rate(format.rate, ValueOr::Nearest).map_err(alsa_err)?;
            hwp.set_period_size(period_frames as _, ValueOr::Nearest)
                .map_err(alsa_err)?;
            // Buffer = 4 periods for smooth playback
            hwp.set_buffer_size((period_frames * 4) as _)
                .map_err(alsa_err)?;
            pcm.hw_params(&hwp).map_err(alsa_err)?;

            let actual_rate = hwp.get_rate().map_err(alsa_err)?;
            let actual_period = hwp.get_period_size().map_err(alsa_err)?;
            let actual_buffer = hwp.get_buffer_size().map_err(alsa_err)?;
            log::info!(
                "ALSA playback configured: rate={}Hz, period={} frames, buffer={} frames",
                actual_rate,
                actual_period,
                actual_buffer
            );
        }

        // Set start threshold to buffer size so playback doesn't start until
        // we have enough data buffered (prevents initial underrun)
        {
            let (buffer_size, _) = pcm.get_params().map_err(alsa_err)?;
            let swp = pcm.sw_params_current().map_err(alsa_err)?;
            swp.set_start_threshold(buffer_size as _).map_err(alsa_err)?;
            pcm.sw_params(&swp).map_err(alsa_err)?;
        }

        pcm.prepare().map_err(alsa_err)?;
        self.pcm = Some(pcm);
        self.current_format = Some(format);
        Ok(())
    }
}

impl AudioSink for AlsaSink {
    fn write_frame(&mut self, pcm_data: &[u8]) -> Result<(), AudioError> {
        let pcm = self
            .pcm
            .as_ref()
            .ok_or_else(|| AudioError::DeviceUnavailable("playback device not started".into()))?;

        // Convert PCM16LE bytes to i16 samples
        let samples: Vec<i16> = pcm_data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let io = pcm.io_i16().map_err(alsa_err)?;

        match io.writei(&samples) {
            Ok(_) => Ok(()),
            Err(e) => {
                // Try to recover from underrun (EPIPE)
                if pcm.try_recover(e, true).is_ok() {
                    log::warn!("ALSA playback: recovered from underrun");
                    // Retry once after recovery
                    io.writei(&samples).map(|_| ()).map_err(alsa_err)
                } else {
                    Err(alsa_err(e))
                }
            }
        }
    }

    fn start(&mut self, format: AudioFormat) -> Result<(), AudioError> {
        // Only reconfigure if format changed or device not open
        if self.pcm.is_some() && self.current_format == Some(format) {
            let pcm = self.pcm.as_ref().unwrap();
            pcm.prepare().map_err(alsa_err)?;
            log::debug!("AlsaSink: restarted (same format)");
        } else {
            self.open_device(format)?;
            log::debug!("AlsaSink: started ({}Hz {}ch)", format.rate, format.channels);
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(ref pcm) = self.pcm {
            // Drain remaining buffered audio before stopping
            if let Err(e) = pcm.drain() {
                log::warn!("ALSA playback drain error: {}", e);
            }
        }
        // Keep the PCM handle open for potential reuse with same format.
        // It will be reconfigured on next start() if format changes.
        log::debug!("AlsaSink: stopped");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alsa_source_new_does_not_open_device() {
        let source = AlsaSource::new("nonexistent", 16000, 1, 640);
        assert!(source.pcm.is_none());
        assert_eq!(source.period_frames, 320);
    }

    #[test]
    fn alsa_sink_new_does_not_open_device() {
        let sink = AlsaSink::new("nonexistent");
        assert!(sink.pcm.is_none());
        assert!(sink.current_format.is_none());
    }

    #[test]
    fn alsa_source_read_without_start_fails() {
        let mut source = AlsaSource::new("nonexistent", 16000, 1, 640);
        let result = source.read_frame();
        assert!(result.is_err());
    }

    #[test]
    fn alsa_sink_write_without_start_fails() {
        let mut sink = AlsaSink::new("nonexistent");
        let result = sink.write_frame(&[0u8; 640]);
        assert!(result.is_err());
    }
}
