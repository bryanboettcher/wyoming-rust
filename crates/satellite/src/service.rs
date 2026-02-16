use std::net::TcpListener;
use std::time::{Duration, Instant};

use wyoming::audio::{AudioChunk, AudioFormat, AudioStart, AudioStop};
use wyoming::event::ProtocolError;
use wyoming::info::SatelliteInfo;
use wyoming::satellite::{Played, StreamingStarted, StreamingStopped};

use crate::config::Config;
use crate::connection::{connect_with_retry, map_server_event, Connection, ConnectionError};
use crate::feedback::{self, Feedback, FanoutFeedback};
use crate::hardware::{AudioError, AudioSink, AudioSource, GpioInput};
use crate::state::{Action, SatelliteInput, SatelliteState};

/// How the satellite establishes its connection to HA.
pub enum ConnectionMode {
    /// Satellite is a TCP server; HA connects in.
    Listen(TcpListener),
    /// Satellite connects out to HA. Connection params are in Config.
    Connect,
}

/// Encapsulates all I/O: connection lifecycle, hardware, and event dispatch.
/// The main loop only sees `SatelliteInput` and `Action` — no sockets, no hardware.
pub struct SatelliteService {
    mode: ConnectionMode,
    conn: Option<Connection>,
    sat_info: SatelliteInfo,
    mic: Box<dyn AudioSource>,
    spk: Box<dyn AudioSink>,
    gpio: Box<dyn GpioInput>,
    feedback: Box<dyn Feedback>,
    audio_format: AudioFormat,
    silence_timeout: Duration,
    last_gpio_high: Instant,
    config: Config,
}

impl SatelliteService {
    /// Build a new SatelliteService from the loaded config.
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let sat_info = SatelliteInfo {
            name: config.satellite.name.clone(),
            area: config.satellite.area.clone(),
            has_mic: true,
            has_snd: config.audio.wav_output.is_some() || config.audio.device.is_some(),
        };

        let mic = create_audio_source(config)?;
        let spk = create_audio_sink(config)?;
        let gpio = create_gpio(config);
        let feedback = create_feedback(config);

        let mode = if config.server.mode == "listen" {
            let addr = format!("{}:{}", config.server.host, config.server.port);
            let listener = TcpListener::bind(&addr)?;
            log::info!("Listening on {}", addr);
            ConnectionMode::Listen(listener)
        } else {
            ConnectionMode::Connect
        };

        let audio_format = config.audio.audio_format();
        let silence_timeout = Duration::from_millis(config.gpio.silence_timeout_ms);

        Ok(Self {
            mode,
            conn: None,
            sat_info,
            mic,
            spk,
            gpio,
            feedback,
            audio_format,
            silence_timeout,
            last_gpio_high: Instant::now(),
            config: config.clone(),
        })
    }

    /// Block until a connection is established and the describe/info + run-satellite
    /// handshake completes. On success, the service is ready for a session.
    pub fn establish_connection(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let conn = match &self.mode {
            ConnectionMode::Listen(listener) => {
                log::info!("Waiting for client connection...");
                let (stream, addr) = listener.accept()?;
                log::info!("Client connected from {}", addr);
                Connection::from_stream(stream, &self.sat_info)?
            }
            ConnectionMode::Connect => {
                connect_with_retry(&self.config.server, &self.sat_info, None)?
            }
        };
        self.conn = Some(conn);
        self.conn.as_mut().unwrap().wait_for_run()?;
        Ok(())
    }

    /// Get the next state-machine input based on current state.
    ///
    /// Returns `Ok(None)` when there is no event to process yet (e.g. audio-chunk
    /// in Responding that was routed directly to speaker). The caller should `continue`.
    pub fn next_input(
        &mut self,
        state: &SatelliteState,
    ) -> Result<Option<SatelliteInput>, Box<dyn std::error::Error>> {
        let conn = self.conn.as_mut().expect("no connection established");

        match state {
            // IDLE: poll GPIO and server socket
            SatelliteState::Idle => {
                // Check for GPIO trigger
                if self.gpio.wait_for_high(Duration::from_millis(100)) {
                    self.last_gpio_high = Instant::now();
                    return Ok(Some(SatelliteInput::GpioHigh));
                }

                // Check for server messages (non-blocking)
                match conn.try_read_event() {
                    Ok(Some(event)) => {
                        if let Some(input) = map_server_event(&event) {
                            return Ok(Some(input));
                        }
                    }
                    Ok(None) => {}
                    Err(ConnectionError::Protocol(ProtocolError::ConnectionClosed)) => {
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                    Err(e) => {
                        log::warn!("Error reading from server in Idle: {}", e);
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                }

                Ok(None)
            }

            // STREAMING / TRIGGERED / PROCESSING: mic read is the clock tick
            SatelliteState::Streaming
            | SatelliteState::Triggered
            | SatelliteState::Processing => {
                // 1. Read one mic frame (~20ms block)
                let frame = match self.mic.read_frame() {
                    Ok(f) => Some(f),
                    Err(AudioError::EndOfStream) => {
                        if matches!(state, SatelliteState::Streaming) {
                            // In Streaming: audio exhausted = silence timeout
                            log::info!("Audio source exhausted, triggering silence timeout");
                            return Ok(Some(SatelliteInput::SilenceTimeout));
                        }
                        // In Triggered/Processing: audio exhausted but server is
                        // still working. Block on server socket instead.
                        log::debug!("Audio source exhausted, waiting for server response");
                        let event = match conn.read_event() {
                            Ok(e) => e,
                            Err(ConnectionError::Protocol(ProtocolError::ConnectionClosed)) => {
                                return Ok(Some(SatelliteInput::Disconnected))
                            }
                            Err(e) => {
                                log::warn!("Error reading from server: {}", e);
                                return Ok(Some(SatelliteInput::Disconnected));
                            }
                        };
                        if let Some(input) = map_server_event(&event) {
                            return Ok(Some(input));
                        }
                        return Ok(None);
                    }
                    Err(e) => return Err(e.into()),
                };

                // 2. Send audio chunk to server
                if let Some(frame) = frame {
                    let chunk = AudioChunk {
                        format: self.audio_format,
                        audio: frame,
                        timestamp: None,
                    };
                    conn.send(chunk)?;
                }

                // 3. Check GPIO for silence timeout
                if self.gpio.is_high() {
                    self.last_gpio_high = Instant::now();
                } else if self.last_gpio_high.elapsed() > self.silence_timeout {
                    return Ok(Some(SatelliteInput::SilenceTimeout));
                }

                // 4. Check for server message (non-blocking)
                match conn.try_read_event() {
                    Ok(Some(event)) => {
                        if let Some(input) = map_server_event(&event) {
                            return Ok(Some(input));
                        }
                    }
                    Ok(None) => {}
                    Err(ConnectionError::Protocol(ProtocolError::ConnectionClosed)) => {
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                    Err(e) => {
                        log::warn!("Error reading from server: {}", e);
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                }

                Ok(None) // No state transition, loop back for next frame
            }

            // RESPONDING: block on server socket for TTS audio
            SatelliteState::Responding => {
                let event = match conn.read_event() {
                    Ok(e) => e,
                    Err(ConnectionError::Protocol(ProtocolError::ConnectionClosed)) => {
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                    Err(e) => {
                        log::warn!("Error reading TTS data: {}", e);
                        return Ok(Some(SatelliteInput::Disconnected));
                    }
                };

                // Handle audio chunks directly (data routing, not state transition)
                if event.event_type == "audio-chunk" {
                    if let Some(ref payload) = event.payload {
                        if let Err(e) = self.spk.write_frame(payload) {
                            log::warn!("Error writing TTS audio to speaker: {}", e);
                        }
                    }
                    return Ok(None); // Stay in Responding, no state transition
                }

                // Map other events to state machine inputs
                if let Some(input) = map_server_event(&event) {
                    return Ok(Some(input));
                }

                Ok(None)
            }
        }
    }

    /// Execute a single state-machine action.
    ///
    /// All actions except `Action::Reconnect` are handled here. Reconnect is
    /// detected by the main loop and triggers session teardown instead.
    pub fn execute(&mut self, action: &Action) -> Result<(), Box<dyn std::error::Error>> {
        match action {
            Action::StartCapture => {
                log::debug!("Action: StartCapture");
                self.mic.start()?;
            }
            Action::StopCapture => {
                log::debug!("Action: StopCapture");
                self.mic.stop()?;
            }
            Action::SendAudioStart => {
                log::debug!("Action: SendAudioStart");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(AudioStart {
                    format: self.audio_format,
                    timestamp: None,
                })?;
            }
            Action::SendAudioStop => {
                log::debug!("Action: SendAudioStop");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(AudioStop { timestamp: None })?;
            }
            Action::SendStreamingStarted => {
                log::debug!("Action: SendStreamingStarted");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(StreamingStarted)?;
            }
            Action::SendStreamingStopped => {
                log::debug!("Action: SendStreamingStopped");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(StreamingStopped)?;
            }
            Action::StartPlayback => {
                log::debug!("Action: StartPlayback");
                self.spk.start()?;
            }
            Action::StopPlayback => {
                log::debug!("Action: StopPlayback");
                self.spk.stop()?;
            }
            Action::SendPlayed => {
                log::debug!("Action: SendPlayed");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(Played)?;
            }
            Action::SetFeedback(state) => {
                log::debug!("Action: SetFeedback({:?})", state);
                self.feedback.update(*state);
            }
            Action::Reconnect => {
                // Should not be called — main loop handles this by returning from session.
                log::warn!("Action::Reconnect reached service.execute(); this is a bug");
            }
        }
        Ok(())
    }

    /// Cleanly shut down all feedback providers.
    pub fn shutdown(&mut self) {
        self.feedback.shutdown();
    }
}

// ============================================================================
// Hardware Factory Functions
// ============================================================================

fn create_audio_source(
    config: &Config,
) -> Result<Box<dyn AudioSource>, Box<dyn std::error::Error>> {
    if let Some(ref wav_path) = config.audio.wav_input {
        let source = crate::hardware::WavFileSource::open(
            wav_path,
            config.audio.frame_size(),
            config.audio.chunk_ms,
        )?;
        Ok(Box::new(source))
    } else {
        Err("ALSA audio source not yet implemented. Use wav_input for testing.".into())
    }
}

fn create_audio_sink(config: &Config) -> Result<Box<dyn AudioSink>, Box<dyn std::error::Error>> {
    if let Some(ref wav_path) = config.audio.wav_output {
        Ok(Box::new(crate::hardware::WavFileSink::new(
            wav_path,
            config.audio.rate,
            config.audio.channels,
        )))
    } else {
        Ok(Box::new(crate::hardware::NullSink))
    }
}

fn create_gpio(config: &Config) -> Box<dyn GpioInput> {
    if config.gpio.auto_trigger || config.gpio.vad_pin.is_none() {
        Box::new(crate::hardware::AutoGpio::new())
    } else {
        log::warn!("Real GPIO not implemented, falling back to auto-trigger");
        Box::new(crate::hardware::AutoGpio::new())
    }
}

fn create_feedback(config: &Config) -> Box<dyn Feedback> {
    let mut fanout = FanoutFeedback::new();

    if config.feedback.is_empty() {
        log::info!("No feedback providers configured, using logging only");
        fanout.add_provider("log", feedback::logging_worker);
    } else {
        for fb_config in &config.feedback {
            match fb_config.method.as_str() {
                "log" => {
                    fanout.add_provider("log", feedback::logging_worker);
                }
                other => {
                    log::warn!(
                        "Unknown feedback method '{}', skipping. Available: log",
                        other
                    );
                }
            }
        }
    }

    Box::new(fanout)
}
