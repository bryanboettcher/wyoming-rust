use std::collections::VecDeque;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use wyoming::audio::{AudioChunk, AudioFormat, AudioStart, AudioStop};
use wyoming::event::{Eventable, ProtocolError};
use wyoming::info::{Attribution, MicProgram, SatelliteInfo, SndProgram};
use wyoming::pipeline::{PipelineStage, RunPipeline};
use wyoming::satellite::{Played, StreamingStarted, StreamingStopped};

use crate::config::Config;
use crate::connection::{connect_with_retry, map_server_event, Connection, ConnectionError, HandshakeResult};
use crate::diagnostics::SharedDiagnostics;
use crate::feedback::{self, Feedback, FanoutFeedback};
use crate::hardware::{AudioError, AudioSink, AudioSource, Vad};
use crate::state::{Action, FeedbackState, SatelliteInput, SatelliteState};

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
    mic_programs: Vec<MicProgram>,
    snd_programs: Vec<SndProgram>,
    mic: Box<dyn AudioSource>,
    spk: Box<dyn AudioSink>,
    vad: Box<dyn Vad>,
    feedback: Box<dyn Feedback>,
    audio_format: AudioFormat,
    /// Format from the most recent server `audio-start` event (TTS playback).
    /// Defaults to the capture format; updated when the server sends audio-start.
    playback_format: AudioFormat,
    silence_timeout: Duration,
    /// Whether the mic is currently running (for continuous-capture VAD modes).
    mic_running: bool,
    /// Timestamp of last voice activity detection.
    last_vad_active: Instant,
    /// Ring buffer of recent audio frames for pre-roll before wake word.
    preroll_buffer: VecDeque<Vec<u8>>,
    /// Maximum number of frames in the pre-roll buffer.
    preroll_max_frames: usize,
    config: Config,
}

impl SatelliteService {
    /// Build a new SatelliteService from the loaded config.
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let sat_info = SatelliteInfo {
            name: config.satellite.name.clone(),
            attribution: Attribution::default(),
            installed: true,
            area: config.satellite.area.clone(),
            description: Some(config.satellite.name.clone()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            has_vad: Some(true),
            active_wake_words: None,
            max_active_wake_words: None,
            supports_trigger: Some(false),
        };

        let audio_format = config.audio.audio_format();

        let mic_programs = vec![MicProgram {
            name: config.satellite.name.clone(),
            attribution: Attribution::default(),
            installed: true,
            description: None,
            version: Some(env!("CARGO_PKG_VERSION").into()),
            mic_format: audio_format,
        }];

        let has_snd = config.audio.playback_device.is_some()
            || config.audio.wav_output.is_some()
            || config.audio.device.is_some();
        let snd_programs = if has_snd {
            vec![SndProgram {
                name: config.satellite.name.clone(),
                attribution: Attribution::default(),
                installed: true,
                description: None,
                version: Some(env!("CARGO_PKG_VERSION").into()),
                snd_format: audio_format,
            }]
        } else {
            vec![]
        };

        let mic = create_audio_source(config)?;
        let spk = create_audio_sink(config)?;
        let vad = create_vad(config)?;
        let feedback = create_feedback(config);

        let mode = if config.server.mode == "listen" {
            let addr = format!("{}:{}", config.server.host, config.server.port);
            let listener = TcpListener::bind(&addr)?;
            log::info!("Listening on {}", addr);
            ConnectionMode::Listen(listener)
        } else {
            ConnectionMode::Connect
        };

        let silence_timeout = Duration::from_millis(config.vad.silence_timeout_ms());

        // Compute pre-roll buffer capacity from buffer_seconds and chunk_ms
        let buffer_seconds = config.vad.buffer_seconds();
        let preroll_max_frames = if buffer_seconds > 0.0 && config.audio.chunk_ms > 0 {
            let chunk_secs = config.audio.chunk_ms as f64 / 1000.0;
            (buffer_seconds / chunk_secs).ceil() as usize
        } else {
            0
        };

        Ok(Self {
            mode,
            conn: None,
            sat_info,
            mic_programs,
            snd_programs,
            mic,
            spk,
            vad,
            feedback,
            audio_format,
            playback_format: audio_format,
            silence_timeout,
            mic_running: false,
            last_vad_active: Instant::now(),
            preroll_buffer: VecDeque::with_capacity(preroll_max_frames),
            preroll_max_frames,
            config: config.clone(),
        })
    }

    /// Block until a connection is established and the handshake completes.
    ///
    /// Handles two HA connection patterns:
    /// 1. HA sends `describe` → we reply with `info` → wait for `run-satellite`
    /// 2. HA sends `run-satellite` directly → skip to main loop
    ///
    /// On success, the service is ready for a session.
    pub fn establish_connection(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Clear any stale pre-roll frames from a prior session so they are not
        // flushed to the new connection on the next VoiceDetected transition.
        self.preroll_buffer.clear();

        let (conn, handshake) = match &self.mode {
            ConnectionMode::Listen(listener) => {
                log::info!("Waiting for client connection...");
                let (stream, addr) = listener.accept()?;
                log::info!("Client connected from {}", addr);
                Connection::from_stream(stream, &self.sat_info, &self.mic_programs, &self.snd_programs)?
            }
            ConnectionMode::Connect => {
                connect_with_retry(&self.config.server, &self.sat_info, &self.mic_programs, &self.snd_programs, None)?
            }
        };
        self.conn = Some(conn);

        if handshake == HandshakeResult::NeedRunSatellite {
            self.conn.as_mut().unwrap().wait_for_run()?;
        }

        Ok(())
    }

    /// Non-blocking poll for a server event, handling transport-level events
    /// (ping/pong, describe) internally. Returns `Ok(Some(event))` only for
    /// application-level events that the caller should process.
    fn poll_server_event(&mut self) -> Result<Option<wyoming::event::Event>, ConnectionError> {
        let conn = self.conn.as_mut().expect("no connection established");
        let event = match conn.try_read_event()? {
            Some(e) => e,
            None => return Ok(None),
        };
        conn.handle_transport_event(event, &self.sat_info, &self.mic_programs, &self.snd_programs)
    }

    /// Blocking read for a server event, handling transport-level events
    /// (ping/pong, describe) internally. Returns `Ok(Some(event))` only for
    /// application-level events. Loops internally if a transport event was handled.
    fn read_server_event(&mut self) -> Result<Option<wyoming::event::Event>, ConnectionError> {
        loop {
            let conn = self.conn.as_mut().expect("no connection established");
            let event = conn.read_event()?;
            let conn = self.conn.as_mut().expect("no connection established");
            match conn.handle_transport_event(event, &self.sat_info, &self.mic_programs, &self.snd_programs)? {
                Some(event) => return Ok(Some(event)),
                None => continue, // transport event handled, read next
            }
        }
    }

    /// Intercept server events to capture playback format, then delegate to
    /// `map_server_event()` for state machine input mapping.
    fn handle_server_event(
        &mut self,
        event: &wyoming::event::Event,
    ) -> Option<SatelliteInput> {
        // Capture AudioFormat from audio-start events for playback configuration
        if event.event_type == AudioStart::EVENT_TYPE {
            match AudioStart::from_event(event.clone()) {
                Ok(start) => {
                    log::info!(
                        "TTS playback format: {}Hz {}bit {}ch",
                        start.format.rate,
                        start.format.width * 8,
                        start.format.channels
                    );
                    self.playback_format = start.format;
                }
                Err(e) => {
                    log::warn!("Failed to parse audio-start format: {}, using default", e);
                }
            }
        }

        map_server_event(event)
    }

    /// Get the next state-machine input based on current state.
    ///
    /// Returns `Ok(None)` when there is no event to process yet (e.g. audio-chunk
    /// in Responding that was routed directly to speaker). The caller should `continue`.
    pub fn next_input(
        &mut self,
        state: &SatelliteState,
    ) -> Result<Option<SatelliteInput>, Box<dyn std::error::Error>> {
        match state {
            // IDLE: either poll GPIO or read mic frames (depending on VAD mode)
            SatelliteState::Idle => {
                if self.vad.needs_continuous_capture() {
                    // Energy/AlwaysOn: mic must be running to detect voice
                    if !self.mic_running {
                        if let Err(e) = self.mic.start() {
                            log::error!("Failed to start mic for continuous VAD: {}", e);
                            return Err(e.into());
                        }
                        self.mic_running = true;
                        log::debug!("Mic started for continuous-capture VAD");
                    }

                    // Read a frame, poll VAD first (borrow), then move into pre-roll buffer
                    match self.mic.read_frame() {
                        Ok(frame) => {
                            // Poll VAD before buffering so we can borrow frame without cloning
                            if self.vad.poll(Some(&frame)) {
                                self.last_vad_active = Instant::now();
                                // Still buffer the frame so FlushPreRollBuffer includes it
                                if self.preroll_max_frames > 0 {
                                    if self.preroll_buffer.len() >= self.preroll_max_frames {
                                        self.preroll_buffer.pop_front();
                                    }
                                    self.preroll_buffer.push_back(frame);
                                }
                                return Ok(Some(SatelliteInput::VoiceDetected));
                            }

                            // No voice detected: move frame into ring buffer (evict oldest if full)
                            if self.preroll_max_frames > 0 {
                                if self.preroll_buffer.len() >= self.preroll_max_frames {
                                    self.preroll_buffer.pop_front();
                                }
                                self.preroll_buffer.push_back(frame);
                            }
                        }
                        Err(AudioError::EndOfStream) => {
                            // WAV file exhausted in Idle — trigger silence timeout for graceful exit
                            log::info!("Audio source exhausted in Idle, triggering silence timeout");
                            return Ok(Some(SatelliteInput::SilenceTimeout));
                        }
                        Err(e) => return Err(e.into()),
                    }

                    // Check for server messages (non-blocking)
                    match self.poll_server_event() {
                        Ok(Some(event)) => {
                            if let Some(input) = self.handle_server_event(&event) {
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
                } else {
                    // GPIO mode: poll without audio
                    if self.vad.poll(None) {
                        self.last_vad_active = Instant::now();
                        return Ok(Some(SatelliteInput::VoiceDetected));
                    }

                    // Sleep briefly to avoid busy-waiting on GPIO poll
                    std::thread::sleep(Duration::from_millis(100));

                    // Check for server messages (non-blocking)
                    match self.poll_server_event() {
                        Ok(Some(event)) => {
                            if let Some(input) = self.handle_server_event(&event) {
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
                        match self.read_server_event() {
                            Ok(Some(event)) => {
                                if let Some(input) = self.handle_server_event(&event) {
                                    return Ok(Some(input));
                                }
                            }
                            Ok(None) => {}
                            Err(ConnectionError::Protocol(ProtocolError::ConnectionClosed)) => {
                                return Ok(Some(SatelliteInput::Disconnected))
                            }
                            Err(e) => {
                                log::warn!("Error reading from server: {}", e);
                                return Ok(Some(SatelliteInput::Disconnected));
                            }
                        }
                        return Ok(None);
                    }
                    Err(e) => return Err(e.into()),
                };

                // 2. Check VAD for silence timeout (before sending, to avoid move)
                let vad_active = if let Some(ref f) = frame {
                    self.vad.poll(Some(f))
                } else {
                    false
                };

                if vad_active {
                    self.last_vad_active = Instant::now();
                } else if self.last_vad_active.elapsed() > self.silence_timeout {
                    return Ok(Some(SatelliteInput::SilenceTimeout));
                }

                // 3. Send audio chunk to server
                if let Some(frame) = frame {
                    let chunk = AudioChunk {
                        format: self.audio_format,
                        audio: frame,
                        timestamp: None,
                    };
                    let conn = self.conn.as_mut().expect("no connection established");
                    conn.send(chunk)?;
                }

                // 4. Check for server message (non-blocking)
                match self.poll_server_event() {
                    Ok(Some(event)) => {
                        if let Some(input) = self.handle_server_event(&event) {
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
                let event = match self.read_server_event() {
                    Ok(Some(event)) => event,
                    Ok(None) => return Ok(None), // transport event handled (e.g. ping)
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
                if let Some(input) = self.handle_server_event(&event) {
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
                if !self.mic_running {
                    self.mic.start()?;
                    self.mic_running = true;
                } else {
                    log::debug!("Mic already running, skipping start");
                }
            }
            Action::StopCapture => {
                if self.vad.needs_continuous_capture() {
                    // Keep mic running for pre-roll buffer; just reset VAD state
                    log::debug!("Action: StopCapture (continuous VAD: mic stays running, VAD reset)");
                    self.vad.reset();
                } else {
                    log::debug!("Action: StopCapture");
                    self.mic.stop()?;
                    self.mic_running = false;
                    self.vad.reset();
                }
            }
            Action::SendRunPipeline => {
                log::debug!("Action: SendRunPipeline");
                let conn = self.conn.as_mut().expect("no connection established");
                // start_stage is Wake because this satellite streams raw audio to the
                // server for openWakeWord detection — the server handles wake word
                // recognition and must therefore start the pipeline from the Wake stage.
                // This differs from the Python reference satellite which performs local
                // wake word detection and sends start_stage: Asr (skipping the server's
                // wake stage entirely).
                conn.send(RunPipeline {
                    start_stage: PipelineStage::Wake,
                    end_stage: PipelineStage::Tts,
                    wake_word_name: None,
                    restart_on_end: false,
                    wake_word_names: None,
                    announce_text: None,
                })?;
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
            Action::FlushPreRollBuffer => {
                let frame_count = self.preroll_buffer.len();
                log::debug!("Action: FlushPreRollBuffer ({} frames)", frame_count);
                let conn = self.conn.as_mut().expect("no connection established");
                for frame in self.preroll_buffer.drain(..) {
                    conn.send(AudioChunk {
                        format: self.audio_format,
                        audio: frame,
                        timestamp: None,
                    })?;
                }
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
                log::debug!("Action: StartPlayback (format: {}Hz {}ch)",
                    self.playback_format.rate, self.playback_format.channels);
                self.spk.start(self.playback_format)?;
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

    /// Update shared diagnostics with current service state.
    ///
    /// Copies a handful of scalars from the service into the mutex — no heap
    /// allocation. Called once per main-loop tick.
    pub fn update_diagnostics(
        &mut self,
        diag: &SharedDiagnostics,
        state: &SatelliteState,
        feedback_state: &FeedbackState,
    ) {
        let mut d = diag.lock().unwrap();

        // Apply pending thresholds from HTTP API
        if let Some(t) = d.pending_attack_threshold.take() {
            self.vad.set_attack_threshold(t);
            d.vad_attack_threshold = Some(t);
            log::info!("VAD attack threshold updated to {} via HTTP", t);
        }
        if let Some(t) = d.pending_sustain_threshold.take() {
            self.vad.set_sustain_threshold(t);
            d.vad_sustain_threshold = Some(t);
            log::info!("VAD sustain threshold updated to {} via HTTP", t);
        }

        d.state = state.clone();
        d.feedback_state = *feedback_state;
        d.connected = self.conn.is_some();
        d.vad_current_energy = self.vad.last_energy();
        d.vad_phase = self.vad.phase().map(|s| s.to_string());
        if let Some(conn) = &self.conn {
            d.last_ping_received = conn.last_ping_received;
        }
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
    } else if let Some(ref device) = config.audio.device {
        let source = crate::hardware::alsa::AlsaSource::new(
            device,
            config.audio.rate,
            config.audio.channels,
            config.audio.frame_size(),
        );
        Ok(Box::new(source))
    } else {
        Err("audio config must specify either 'device' or 'wav_input'".into())
    }
}

fn create_audio_sink(config: &Config) -> Result<Box<dyn AudioSink>, Box<dyn std::error::Error>> {
    if let Some(ref device) = config.audio.playback_device {
        Ok(Box::new(crate::hardware::alsa::AlsaSink::new(device)))
    } else if let Some(ref wav_path) = config.audio.wav_output {
        Ok(Box::new(crate::hardware::WavFileSink::new(
            wav_path,
            config.audio.rate,
            config.audio.channels,
        )))
    } else {
        Ok(Box::new(crate::hardware::NullSink))
    }
}

fn create_vad(config: &Config) -> Result<Box<dyn Vad>, Box<dyn std::error::Error>> {
    use crate::config::VadConfig;

    match &config.vad {
        VadConfig::AlwaysOn { .. } => {
            log::info!("VAD mode: AlwaysOn");
            Ok(Box::new(crate::hardware::AlwaysOnVad::new()))
        }
        VadConfig::Gpio { pin, .. } => {
            log::info!("VAD mode: GPIO (pin {})", pin);
            match crate::hardware::GpioVad::new(*pin) {
                Ok(vad) => Ok(Box::new(vad)),
                Err(e) => {
                    log::warn!("GPIO VAD unavailable ({}), falling back to AlwaysOn", e);
                    Ok(Box::new(crate::hardware::AlwaysOnVad::new()))
                }
            }
        }
        VadConfig::Energy { attack_threshold, sustain_threshold, .. } => {
            log::info!("VAD mode: Energy (attack={}, sustain={})", attack_threshold, sustain_threshold);
            Ok(Box::new(crate::hardware::EnergyVad::new(*attack_threshold, *sustain_threshold)))
        }
    }
}

fn create_feedback(config: &Config) -> Box<dyn Feedback> {
    use crate::config::FeedbackProviderConfig;

    let mut fanout = FanoutFeedback::new();

    if config.feedback.is_empty() {
        log::info!("No feedback providers configured, using logging only");
        fanout.add_provider("log", feedback::logging_worker);
    } else {
        for fb_config in &config.feedback {
            match fb_config {
                FeedbackProviderConfig::Log {} => {
                    fanout.add_provider("log", feedback::logging_worker);
                }
                FeedbackProviderConfig::Console { output, states } => {
                    let output = output.clone();
                    let states = states.clone();
                    fanout.add_provider("console", move |rx| {
                        feedback::console::worker(rx, &output, &states);
                    });
                }
                FeedbackProviderConfig::Pwm { pin, states } => {
                    let pin = *pin;
                    let states = states.clone();
                    fanout.add_provider("pwm", move |rx| {
                        feedback::pwm::worker(rx, pin, &states);
                    });
                }
                FeedbackProviderConfig::Neopixel {
                    pin,
                    led_count,
                    spi_device,
                    states,
                } => {
                    let pin = *pin;
                    let led_count = *led_count;
                    let spi_device = spi_device.clone();
                    let states = states.clone();
                    fanout.add_provider("neopixel", move |rx| {
                        feedback::neopixel::worker(rx, pin, led_count, spi_device.as_deref(), &states);
                    });
                }
            }
        }
    }

    Box::new(fanout)
}
