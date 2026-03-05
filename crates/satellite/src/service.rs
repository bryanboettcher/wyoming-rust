use std::collections::VecDeque;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use wyoming::audio::{AudioChunk, AudioFormat, AudioStart, AudioStop};
use wyoming::event::{Eventable, ProtocolError};
use wyoming::info::{Attribution, MicProgram, SatelliteInfo, SndProgram};
use wyoming::pipeline::{PipelineStage, RunPipeline};
use wyoming::satellite::{Played, StreamingStarted, StreamingStopped};

use crate::config::Config;
use crate::pipeline;
use crate::connection::{connect_with_retry, map_server_event, Connection, ConnectionError, HandshakeResult};
use crate::diagnostics::{push_sse, SharedDiagnostics, SseClients};
use crate::feedback::{self, Feedback, FanoutFeedback};
use crate::hardware::{AudioError, AudioSink, AudioSource};
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
    /// Timestamp when the current streaming session started, for watchdog.
    streaming_started: Instant,
    /// Maximum streaming duration before forced timeout. Duration::ZERO = no limit.
    max_stream_duration: Duration,
    /// Ring buffer of recent audio frames for pre-roll before wake word.
    preroll_buffer: VecDeque<Vec<u8>>,
    /// Maximum number of frames in the pre-roll buffer.
    preroll_max_frames: usize,
    config: Config,
    /// SSE client registry for real-time streaming of tick data and protocol events.
    sse_clients: Option<SseClients>,
    /// Timestamp set right after read_frame() returns (or at next_input() entry for
    /// non-audio states). Used by update_diagnostics() to compute non-ALSA work time.
    tick_post_read: Instant,
    runner: Box<dyn pipeline::Runner>,
    sample_buf: Vec<i16>,
    /// Counter for subsampling mel spectrogram SSE events (send every 5th frame = 10Hz).
    mel_sse_counter: u32,
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
        let feedback = create_feedback(config);

        let mode = if config.server.mode == "listen" {
            let addr = format!("{}:{}", config.server.host, config.server.port);
            let listener = TcpListener::bind(&addr)?;
            log::info!("Listening on {}", addr);
            ConnectionMode::Listen(listener)
        } else {
            ConnectionMode::Connect
        };

        let silence_timeout = Duration::from_millis(config.pipeline.silence_timeout_ms);

        // Compute pre-roll buffer capacity from buffer_seconds and chunk_ms
        let buffer_seconds = config.pipeline.buffer_seconds;
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
            feedback,
            audio_format,
            playback_format: audio_format,
            silence_timeout,
            mic_running: false,
            last_vad_active: Instant::now(),
            streaming_started: Instant::now(),
            max_stream_duration: if config.pipeline.max_stream_seconds > 0 {
                Duration::from_secs(config.pipeline.max_stream_seconds)
            } else {
                Duration::ZERO
            },
            preroll_buffer: VecDeque::with_capacity(preroll_max_frames),
            preroll_max_frames,
            config: config.clone(),
            sse_clients: None,
            tick_post_read: Instant::now(),
            runner: Box::new(pipeline::SimpleRunner::new(&config.pipeline, config.audio.rate)),
            sample_buf: Vec::new(),
            mel_sse_counter: 0,
        })
    }

    /// Register SSE client list for real-time event streaming.
    pub fn set_sse_clients(&mut self, clients: SseClients) {
        self.sse_clients = Some(clients);
    }

    /// Returns pipeline stage names for diagnostics snapshot.
    pub fn stage_names(&self) -> Vec<&'static str> {
        self.runner.stage_names()
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
        // Push incoming protocol event to SSE clients (skip audio-chunk — too frequent)
        if event.event_type != "audio-chunk" {
            if let Some(ref clients) = self.sse_clients {
                let summary = summarize_event(event);
                let json = format!(
                    r#"{{"dir":"in","type":"{}","summary":"{}"}}"#,
                    event.event_type, summary
                );
                push_sse(clients, "protocol", &json);
            }
        }

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
        // Default: measure from call entry (for non-audio states like Responding).
        // Overwritten after read_frame() for audio states to exclude ALSA blocking time.
        self.tick_post_read = Instant::now();

        match state {
            SatelliteState::Idle => {
                // Mic always runs in idle for pipeline-based detection
                if !self.mic_running {
                    if let Err(e) = self.mic.start() {
                        log::error!("Failed to start mic: {}", e);
                        return Err(e.into());
                    }
                    self.mic_running = true;
                    log::debug!("Mic started for pipeline detection");
                }

                match self.mic.read_frame() {
                    Ok(mut frame) => {
                        self.tick_post_read = Instant::now();

                        // Run audio through pipeline (filters + detection)
                        pipeline::pcm16le_to_samples(&frame, &mut self.sample_buf);
                        let triggered = self.runner.process_frame(&mut self.sample_buf);
                        pipeline::samples_to_pcm16le(&self.sample_buf, &mut frame);

                        if triggered {
                            self.last_vad_active = Instant::now();
                            if self.preroll_max_frames > 0 {
                                if self.preroll_buffer.len() >= self.preroll_max_frames {
                                    self.preroll_buffer.pop_front();
                                }
                                self.preroll_buffer.push_back(frame);
                            }
                            return Ok(Some(SatelliteInput::VoiceDetected));
                        }

                        // No voice detected: buffer for pre-roll
                        if self.preroll_max_frames > 0 {
                            if self.preroll_buffer.len() >= self.preroll_max_frames {
                                self.preroll_buffer.pop_front();
                            }
                            self.preroll_buffer.push_back(frame);
                        }
                    }
                    Err(AudioError::EndOfStream) => {
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

                Ok(None)
            }

            // STREAMING / TRIGGERED / PROCESSING: mic read is the clock tick
            SatelliteState::Streaming
            | SatelliteState::Triggered
            | SatelliteState::Processing => {
                // 1. Read one mic frame (~20ms block)
                let (frame, vad_active) = match self.mic.read_frame() {
                    Ok(mut f) => {
                        self.tick_post_read = Instant::now();

                        // Run audio through pipeline (filters + detection)
                        pipeline::pcm16le_to_samples(&f, &mut self.sample_buf);
                        let triggered = self.runner.process_frame(&mut self.sample_buf);
                        pipeline::samples_to_pcm16le(&self.sample_buf, &mut f);

                        (Some(f), triggered)
                    }
                    Err(AudioError::EndOfStream) => {
                        if matches!(state, SatelliteState::Streaming) {
                            log::info!("Audio source exhausted, triggering silence timeout");
                            return Ok(Some(SatelliteInput::SilenceTimeout));
                        }
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

                // 2. Pipeline already ran detection during frame processing above.
                if vad_active {
                    self.last_vad_active = Instant::now();
                } else if self.last_vad_active.elapsed() > self.silence_timeout {
                    return Ok(Some(SatelliteInput::SilenceTimeout));
                }

                // 2b. Watchdog: force stop if streaming for too long (stuck detection).
                if self.max_stream_duration > Duration::ZERO
                    && self.streaming_started.elapsed() > self.max_stream_duration
                {
                    log::warn!(
                        "Max stream duration exceeded ({:.0}s), forcing silence timeout",
                        self.max_stream_duration.as_secs_f64()
                    );
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
                // Mic stays running for pre-roll buffer; just reset pipeline detection state
                log::debug!("Action: StopCapture (pipeline reset, mic stays running)");
                self.runner.reset();
            }
            Action::SendRunPipeline => {
                log::debug!("Action: SendRunPipeline");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(RunPipeline {
                    start_stage: PipelineStage::Wake,
                    end_stage: PipelineStage::Tts,
                    wake_word_name: None,
                    restart_on_end: false,
                    wake_word_names: None,
                    announce_text: None,
                })?;
                self.push_protocol_out("run-pipeline", "start_stage=wake");
            }
            Action::SendAudioStart => {
                log::debug!("Action: SendAudioStart");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(AudioStart {
                    format: self.audio_format,
                    timestamp: None,
                })?;
                self.push_protocol_out("audio-start", "");
            }
            Action::SendAudioStop => {
                log::debug!("Action: SendAudioStop");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(AudioStop { timestamp: None })?;
                self.push_protocol_out("audio-stop", "");
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
                self.streaming_started = Instant::now();
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(StreamingStarted)?;
                self.push_protocol_out("streaming-started", "");
            }
            Action::SendStreamingStopped => {
                log::debug!("Action: SendStreamingStopped");
                let conn = self.conn.as_mut().expect("no connection established");
                conn.send(StreamingStopped)?;
                self.push_protocol_out("streaming-stopped", "");
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
                self.push_protocol_out("played", "");
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

        // Apply pending ratio changes from HTTP API
        {
            let pending = self.runner.pending();
            if let Some(r) = d.pending_attack_ratio.take() {
                pending.attack_ratio = Some(r);
                log::info!("Attack ratio {} queued for next analyze cycle", r);
            }
            if let Some(r) = d.pending_sustain_ratio.take() {
                pending.sustain_ratio = Some(r);
                log::info!("Sustain ratio {} queued for next analyze cycle", r);
            }
        }

        if d.pending_reset {
            d.pending_reset = false;
            self.runner.reset();
            log::info!("Pipeline reset via HTTP");
        }

        d.state = state.clone();
        d.feedback_state = *feedback_state;
        d.connected = self.conn.is_some();
        if let Some(conn) = &self.conn {
            d.last_ping_received = conn.last_ping_received;
        }

        // Snapshot pipeline stats (needed for SSE + rollup)
        let snapshot = self.runner.stats_snapshot();

        // Send mel spectrogram SSE event at 10Hz (every 5th frame)
        self.mel_sse_counter += 1;
        if self.mel_sse_counter % 5 == 0 {
            if let (Some(ref clients), Some(ref mel)) = (&self.sse_clients, &snapshot.mel_energies) {
                let mut b64 = String::with_capacity(40);
                base64_encode(mel, &mut b64);
                push_sse(clients, "mel", &b64);
            }
        }

        // Push tick data to SSE clients with structured stages
        if let Some(ref clients) = self.sse_clients {
            let work_us = self.tick_post_read.elapsed().as_micros() as u32;

            // Build structured stages JSON: {"stage_name": {"key": val, ...}, ...}
            let mut stages_json = String::from("{");
            let mut current_stage: Option<&str> = None;
            let mut first_stage = true;
            let mut first_stat = true;
            for &(stage_name, key, val) in &snapshot.entries {
                use std::fmt::Write;
                if Some(stage_name) != current_stage {
                    if current_stage.is_some() {
                        stages_json.push('}');
                    }
                    if !first_stage {
                        stages_json.push(',');
                    }
                    write!(stages_json, r#""{}":{{"#, stage_name).ok();
                    current_stage = Some(stage_name);
                    first_stage = false;
                    first_stat = true;
                }
                if !first_stat {
                    stages_json.push(',');
                }
                write!(stages_json, r#""{}":{:.4}"#, key.name(), val).ok();
                first_stat = false;
            }
            if current_stage.is_some() {
                stages_json.push('}');
            }
            stages_json.push('}');

            let json = format!(
                r#"{{"state":"{:?}","feedback":"{:?}","connected":{},"work_us":{},"stages":{}}}"#,
                state,
                feedback_state,
                d.connected,
                work_us,
                stages_json,
            );
            push_sse(clients, "tick", &json);
        }

        // Feed rollup accumulator from pipeline stats
        {
            use crate::pipeline::StatKey;
            use crate::diagnostics::extract_stat;
            let energy = extract_stat(&snapshot.entries, StatKey::Energy);
            let floor = extract_stat(&snapshot.entries, StatKey::NoiseFloor);
            let triggered = extract_stat(&snapshot.entries, StatKey::Triggered) > 0.5;
            let zcr = extract_stat(&snapshot.entries, StatKey::ZeroCrossingRate);
            let speech_band = extract_stat(&snapshot.entries, StatKey::SpeechBandRatio);
            let centroid = extract_stat(&snapshot.entries, StatKey::SpectralCentroid);
            let flatness = extract_stat(&snapshot.entries, StatKey::SpectralFlatness);
            d.feed_rollup(energy, floor, triggered, zcr, speech_band, centroid, flatness);
            d.session_tracker.feed_tick(energy, zcr, speech_band, floor);
        }
    }

    /// Push an outgoing protocol event to SSE clients.
    fn push_protocol_out(&self, event_type: &str, summary: &str) {
        if let Some(ref clients) = self.sse_clients {
            let json = format!(
                r#"{{"dir":"out","type":"{}","summary":"{}"}}"#,
                event_type, summary
            );
            push_sse(clients, "protocol", &json);
        }
    }

    /// Cleanly shut down all feedback providers.
    pub fn shutdown(&mut self) {
        self.feedback.shutdown();
    }
}

/// Extract a human-readable summary from an incoming Wyoming event.
fn summarize_event(event: &wyoming::event::Event) -> String {
    match event.event_type.as_str() {
        "detection" => event
            .data
            .get("name")
            .and_then(|v| v.as_str())
            .map(|n| format!("name={}", n))
            .unwrap_or_default(),
        "transcript" => event
            .data
            .get("text")
            .and_then(|v| v.as_str())
            .map(|t| format!("text={}", t))
            .unwrap_or_default(),
        "run-pipeline" => event
            .data
            .get("start_stage")
            .and_then(|v| v.as_str())
            .map(|s| format!("start_stage={}", s))
            .unwrap_or_default(),
        "audio-start" => event
            .data
            .get("rate")
            .and_then(|v| v.as_u64())
            .map(|r| format!("rate={}", r))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

// ============================================================================
// Base64 encoder (minimal, no external dependency)
// ============================================================================

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8], out: &mut String) {
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[((n >> 18) & 63) as usize] as char);
        out.push(B64_CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64_CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64_CHARS[(n & 63) as usize] as char } else { '=' });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encode_empty() {
        let mut out = String::new();
        base64_encode(&[], &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn base64_encode_three_bytes() {
        // "Man" → "TWFu"
        let mut out = String::new();
        base64_encode(b"Man", &mut out);
        assert_eq!(out, "TWFu");
    }

    #[test]
    fn base64_encode_two_bytes_padding() {
        // "Ma" → "TWE="
        let mut out = String::new();
        base64_encode(b"Ma", &mut out);
        assert_eq!(out, "TWE=");
    }

    #[test]
    fn base64_encode_one_byte_padding() {
        // "M" → "TQ=="
        let mut out = String::new();
        base64_encode(b"M", &mut out);
        assert_eq!(out, "TQ==");
    }

    #[test]
    fn base64_encode_26_mel_bytes() {
        let data = vec![128u8; 26];
        let mut out = String::new();
        base64_encode(&data, &mut out);
        // 26 bytes = 8 full chunks (24 bytes) + 1 partial (2 bytes)
        // 8*4 + 4 = 36 chars
        assert_eq!(out.len(), 36);
        assert!(out.ends_with('='), "26 bytes should produce padding");
    }
}
