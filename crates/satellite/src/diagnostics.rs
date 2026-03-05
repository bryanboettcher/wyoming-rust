use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use crate::config::Config;
use crate::pipeline::StatKey;
use crate::state::{FeedbackState, SatelliteState};

// ── Session Tracker ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionOutcome {
    /// Streaming→Idle without reaching Processing (most common false trigger)
    SilenceTimeout,
    /// Reached Processing or Responding
    Completed,
    /// Connection lost during session
    Disconnected,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRecord {
    pub onset_uptime_ms: u64,
    pub duration_ms: u64,
    pub onset_energy: f64,
    pub peak_energy: f64,
    pub onset_zcr: f64,
    pub mean_zcr: f64,
    pub onset_speech_band: f64,
    pub mean_speech_band: f64,
    pub noise_floor: f64,
    pub peak_state: String,
    pub outcome: SessionOutcome,
}

struct ActiveSession {
    onset_uptime_ms: u64,
    onset_energy: f64,
    onset_zcr: f64,
    onset_speech_band: f64,
    noise_floor: f64,
    peak_energy: f64,
    zcr_sum: f64,
    speech_band_sum: f64,
    frame_count: u32,
    peak_state: SatelliteState,
    needs_onset_stats: bool,
}

pub struct SessionTracker {
    sessions: VecDeque<SessionRecord>,
    max_sessions: usize,
    active: Option<ActiveSession>,
}

impl SessionTracker {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: VecDeque::new(),
            max_sessions,
            active: None,
        }
    }

    pub fn start_session(&mut self, uptime_ms: u64) {
        self.active = Some(ActiveSession {
            onset_uptime_ms: uptime_ms,
            onset_energy: 0.0,
            onset_zcr: 0.0,
            onset_speech_band: 0.0,
            noise_floor: 0.0,
            peak_energy: 0.0,
            zcr_sum: 0.0,
            speech_band_sum: 0.0,
            frame_count: 0,
            peak_state: SatelliteState::Streaming,
            needs_onset_stats: true,
        });
    }

    pub fn end_session(&mut self, uptime_ms: u64, outcome: SessionOutcome) {
        if let Some(active) = self.active.take() {
            let duration_ms = uptime_ms.saturating_sub(active.onset_uptime_ms);
            let n = active.frame_count.max(1) as f64;
            let record = SessionRecord {
                onset_uptime_ms: active.onset_uptime_ms,
                duration_ms,
                onset_energy: active.onset_energy,
                peak_energy: active.peak_energy,
                onset_zcr: active.onset_zcr,
                mean_zcr: active.zcr_sum / n,
                onset_speech_band: active.onset_speech_band,
                mean_speech_band: active.speech_band_sum / n,
                noise_floor: active.noise_floor,
                peak_state: format!("{:?}", active.peak_state),
                outcome,
            };
            if self.sessions.len() >= self.max_sessions {
                self.sessions.pop_front();
            }
            self.sessions.push_back(record);
        }
    }

    pub fn feed_tick(&mut self, energy: f64, zcr: f64, speech_band: f64, floor: f64) {
        if let Some(ref mut active) = self.active {
            if active.needs_onset_stats {
                active.onset_energy = energy;
                active.onset_zcr = zcr;
                active.onset_speech_band = speech_band;
                active.noise_floor = floor;
                active.needs_onset_stats = false;
            }
            if energy > active.peak_energy {
                active.peak_energy = energy;
            }
            active.zcr_sum += zcr;
            active.speech_band_sum += speech_band;
            active.frame_count += 1;
        }
    }

    pub fn update_peak_state(&mut self, state: &SatelliteState) {
        if let Some(ref mut active) = self.active {
            // "Higher" state = further along the pipeline
            let rank = |s: &SatelliteState| match s {
                SatelliteState::Idle => 0,
                SatelliteState::Streaming => 1,
                SatelliteState::Triggered => 2,
                SatelliteState::Processing => 3,
                SatelliteState::Responding => 4,
            };
            if rank(state) > rank(&active.peak_state) {
                active.peak_state = state.clone();
            }
        }
    }

    pub fn has_active(&self) -> bool {
        self.active.is_some()
    }

    /// Returns the peak state of the active session, if any.
    pub fn active_peak_state(&self) -> Option<&SatelliteState> {
        self.active.as_ref().map(|a| &a.peak_state)
    }

    pub fn sessions(&self) -> &VecDeque<SessionRecord> {
        &self.sessions
    }
}

const MAX_ROLLUP_ENTRIES: usize = 1440; // 24 hours at 1/min
const ROLLUP_INTERVAL_SECS: u64 = 60;
const MAX_SESSION_RECORDS: usize = 10_000;

/// Single minute rollup entry. Serialized to JSON for `GET /rollups`.
#[derive(Debug, Clone, Serialize)]
pub struct RollupEntry {
    pub uptime_secs: u64,
    pub energy_min: f64,
    pub energy_max: f64,
    pub energy_mean: f64,
    pub energy_p95: f64,
    pub floor_min: f64,
    pub floor_max: f64,
    pub floor_mean: f64,
    pub trigger_count: u32,
    pub triggered_frames: u32,
    pub total_frames: u32,
    pub zcr_mean: f64,
    pub speech_band_mean: f64,
    pub session_count: u32,
    pub false_session_count: u32,
    pub session_dur_mean_ms: u32,
    pub session_dur_max_ms: u32,
    pub spectral_centroid_mean: f64,
    pub spectral_flatness_mean: f64,
}

/// Per-tick accumulator for building minute rollup entries.
pub struct RollupAccumulator {
    energy_sum: f64,
    energy_min: f64,
    energy_max: f64,
    energy_samples: Vec<f64>,
    floor_sum: f64,
    floor_min: f64,
    floor_max: f64,
    trigger_count: u32,
    triggered_frames: u32,
    total_frames: u32,
    was_triggered: bool,
    zcr_sum: f64,
    speech_band_sum: f64,
    session_count: u32,
    false_session_count: u32,
    session_dur_sum_ms: u64,
    session_dur_max_ms: u32,
    spectral_centroid_sum: f64,
    spectral_flatness_sum: f64,
}

impl RollupAccumulator {
    fn new() -> Self {
        let mut energy_samples = Vec::new();
        energy_samples.reserve(3000);
        Self {
            energy_sum: 0.0,
            energy_min: f64::MAX,
            energy_max: f64::MIN,
            energy_samples,
            floor_sum: 0.0,
            floor_min: f64::MAX,
            floor_max: f64::MIN,
            trigger_count: 0,
            triggered_frames: 0,
            total_frames: 0,
            was_triggered: false,
            zcr_sum: 0.0,
            speech_band_sum: 0.0,
            session_count: 0,
            false_session_count: 0,
            session_dur_sum_ms: 0,
            session_dur_max_ms: 0,
            spectral_centroid_sum: 0.0,
            spectral_flatness_sum: 0.0,
        }
    }

    /// Record a completed session into the current rollup interval.
    pub fn record_session(&mut self, duration_ms: u64, outcome: SessionOutcome) {
        self.session_count += 1;
        if outcome == SessionOutcome::SilenceTimeout {
            self.false_session_count += 1;
        }
        self.session_dur_sum_ms += duration_ms;
        let dur_ms = duration_ms.min(u32::MAX as u64) as u32;
        if dur_ms > self.session_dur_max_ms {
            self.session_dur_max_ms = dur_ms;
        }
    }

    /// Feed one tick's worth of stats into the accumulator.
    pub fn feed(&mut self, energy: f64, floor: f64, triggered: bool, zcr: f64, speech_band: f64,
                spectral_centroid: f64, spectral_flatness: f64) {
        self.energy_sum += energy;
        if energy < self.energy_min { self.energy_min = energy; }
        if energy > self.energy_max { self.energy_max = energy; }
        self.energy_samples.push(energy);

        self.floor_sum += floor;
        if floor < self.floor_min { self.floor_min = floor; }
        if floor > self.floor_max { self.floor_max = floor; }

        self.total_frames += 1;
        if triggered {
            self.triggered_frames += 1;
        }
        // Onset detection
        if triggered && !self.was_triggered {
            self.trigger_count += 1;
        }
        self.was_triggered = triggered;

        self.zcr_sum += zcr;
        self.speech_band_sum += speech_band;
        self.spectral_centroid_sum += spectral_centroid;
        self.spectral_flatness_sum += spectral_flatness;
    }

    /// Finalize the minute and produce a rollup entry. Resets the accumulator.
    pub fn finish(&mut self, uptime_secs: u64) -> Option<RollupEntry> {
        if self.total_frames == 0 {
            return None;
        }

        let n = self.total_frames as f64;
        let energy_mean = self.energy_sum / n;

        // P95: sort and index
        self.energy_samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_idx = ((self.energy_samples.len() as f64) * 0.95) as usize;
        let energy_p95 = self.energy_samples.get(p95_idx.min(self.energy_samples.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0);

        let entry = RollupEntry {
            uptime_secs,
            energy_min: self.energy_min,
            energy_max: self.energy_max,
            energy_mean,
            energy_p95,
            floor_min: self.floor_min,
            floor_max: self.floor_max,
            floor_mean: self.floor_sum / n,
            trigger_count: self.trigger_count,
            triggered_frames: self.triggered_frames,
            total_frames: self.total_frames,
            zcr_mean: self.zcr_sum / n,
            speech_band_mean: self.speech_band_sum / n,
            session_count: self.session_count,
            false_session_count: self.false_session_count,
            session_dur_mean_ms: if self.session_count > 0 {
                (self.session_dur_sum_ms / self.session_count as u64).min(u32::MAX as u64) as u32
            } else {
                0
            },
            session_dur_max_ms: self.session_dur_max_ms,
            spectral_centroid_mean: self.spectral_centroid_sum / n,
            spectral_flatness_mean: self.spectral_flatness_sum / n,
        };

        // Reset for next minute
        self.energy_sum = 0.0;
        self.energy_min = f64::MAX;
        self.energy_max = f64::MIN;
        self.energy_samples.clear();
        self.floor_sum = 0.0;
        self.floor_min = f64::MAX;
        self.floor_max = f64::MIN;
        self.trigger_count = 0;
        self.triggered_frames = 0;
        self.total_frames = 0;
        // Preserve was_triggered across minutes for onset tracking
        self.zcr_sum = 0.0;
        self.speech_band_sum = 0.0;
        self.session_count = 0;
        self.false_session_count = 0;
        self.session_dur_sum_ms = 0;
        self.session_dur_max_ms = 0;
        self.spectral_centroid_sum = 0.0;
        self.spectral_flatness_sum = 0.0;

        Some(entry)
    }
}

/// Extract a named stat from a snapshot's entries.
pub fn extract_stat(entries: &[(&str, StatKey, f64)], key: StatKey) -> f64 {
    entries.iter()
        .find(|&&(_, k, _)| k == key)
        .map(|&(_, _, v)| v)
        .unwrap_or(0.0)
}

/// Raw diagnostic data updated by the main loop every tick.
///
/// All fields are cheap scalars or pre-computed strings (set once at startup).
/// The per-tick update copies a handful of values — zero heap allocation.
/// The serializable [`DiagnosticsSnapshot`] is built on demand only when an
/// HTTP request arrives.
pub struct DiagnosticsState {
    // ── Dynamic (updated per-tick) ──────────────────────────────────────
    pub state: SatelliteState,
    pub feedback_state: FeedbackState,
    pub last_state_change: Instant,
    pub connected: bool,
    pub last_ping_received: Option<Instant>,
    pub interaction_count: u64,

    // ── Commands (written by HTTP thread, consumed by main loop) ───────
    pub pending_attack_ratio: Option<f32>,
    pub pending_sustain_ratio: Option<f32>,
    pub pending_reset: bool,

    // ── Minute rollup (written by main loop, read by HTTP) ─────────────
    pub rollup_entries: Vec<RollupEntry>,
    pub rollup_accumulator: RollupAccumulator,
    pub rollup_last_push: Instant,

    // ── Session tracker (written by main loop, read by HTTP) ─────────
    pub session_tracker: SessionTracker,

    // ── Static (set once at startup) ────────────────────────────────────
    started_at: Instant,
    satellite_name: String,
    area: Option<String>,
    server_address: String,
    audio_device: String,
    audio_format: String,
    stage_names: Vec<String>,
}

impl DiagnosticsState {
    pub fn new(config: &Config) -> Self {
        let now = Instant::now();

        let audio_device = config
            .audio
            .device
            .clone()
            .or_else(|| config.audio.wav_input.clone())
            .unwrap_or_else(|| "unknown".into());
        let audio_format = format!(
            "{}Hz {}bit {}ch",
            config.audio.rate,
            config.audio.width * 8,
            config.audio.channels
        );

        Self {
            state: SatelliteState::Idle,
            feedback_state: FeedbackState::Idle,
            last_state_change: now,
            connected: false,
            last_ping_received: None,
            interaction_count: 0,
            pending_attack_ratio: None,
            pending_sustain_ratio: None,
            pending_reset: false,
            rollup_entries: Vec::with_capacity(MAX_ROLLUP_ENTRIES),
            rollup_accumulator: RollupAccumulator::new(),
            rollup_last_push: now,
            session_tracker: SessionTracker::new(MAX_SESSION_RECORDS),
            started_at: now,
            satellite_name: config.satellite.name.clone(),
            area: config.satellite.area.clone(),
            server_address: format!("{}:{}", config.server.host, config.server.port),
            audio_device,
            audio_format,
            stage_names: Vec::new(),
        }
    }

    /// Set the pipeline stage names (called once after runner is created).
    pub fn set_stage_names(&mut self, names: Vec<String>) {
        self.stage_names = names;
    }

    /// Called on every state transition to drive session tracking.
    pub fn on_state_change(&mut self, old: &SatelliteState, new: &SatelliteState) {
        let uptime_ms = self.started_at.elapsed().as_millis() as u64;

        // Idle → Streaming: start a new session
        if *old == SatelliteState::Idle && *new == SatelliteState::Streaming {
            self.session_tracker.start_session(uptime_ms);
            return;
        }

        // Any → Idle while session is active: end it
        if *new == SatelliteState::Idle && self.session_tracker.has_active() {
            let outcome = match self.session_tracker.active_peak_state() {
                Some(SatelliteState::Processing) | Some(SatelliteState::Responding) => SessionOutcome::Completed,
                _ => SessionOutcome::SilenceTimeout,
            };
            self.session_tracker.end_session(uptime_ms, outcome);
            // Record into rollup accumulator
            if let Some(last) = self.session_tracker.sessions().back() {
                self.rollup_accumulator.record_session(last.duration_ms, outcome);
            }
            return;
        }

        // Any other transition during active session: update peak state
        self.session_tracker.update_peak_state(new);
    }

    /// Abort active session as Disconnected (called on session re-entry or error).
    pub fn abort_active_session(&mut self) {
        if self.session_tracker.has_active() {
            let uptime_ms = self.started_at.elapsed().as_millis() as u64;
            self.session_tracker.end_session(uptime_ms, SessionOutcome::Disconnected);
            if let Some(last) = self.session_tracker.sessions().back() {
                self.rollup_accumulator.record_session(last.duration_ms, SessionOutcome::Disconnected);
            }
        }
    }

    /// Feed one tick's pipeline stats into the rollup accumulator.
    /// If a minute has elapsed, finalizes the entry and pushes to the ring buffer.
    pub fn feed_rollup(&mut self, energy: f64, floor: f64, triggered: bool, zcr: f64, speech_band: f64,
                       spectral_centroid: f64, spectral_flatness: f64) {
        self.rollup_accumulator.feed(energy, floor, triggered, zcr, speech_band,
                                     spectral_centroid, spectral_flatness);

        if self.rollup_last_push.elapsed().as_secs() >= ROLLUP_INTERVAL_SECS {
            let uptime_secs = self.started_at.elapsed().as_secs();
            if let Some(entry) = self.rollup_accumulator.finish(uptime_secs) {
                if self.rollup_entries.len() >= MAX_ROLLUP_ENTRIES {
                    self.rollup_entries.remove(0);
                }
                self.rollup_entries.push(entry);
                log::debug!("Rollup entry pushed ({} total)", self.rollup_entries.len());
            }
            self.rollup_last_push = Instant::now();
        }
    }

    fn to_sse_snapshot(&self) -> String {
        let stages_json: Vec<String> = self.stage_names.iter()
            .map(|s| format!(r#""{}""#, s))
            .collect();
        format!(
            r#"{{"satellite_name":"{}","area":{},"audio_device":"{}","audio_format":"{}","server_address":"{}","stages":[{}]}}"#,
            self.satellite_name,
            match &self.area {
                Some(a) => format!(r#""{}""#, a),
                None => "null".into(),
            },
            self.audio_device,
            self.audio_format,
            self.server_address,
            stages_json.join(","),
        )
    }

    /// Build a serializable snapshot on demand (only called on HTTP request).
    fn to_snapshot(&self) -> DiagnosticsSnapshot {
        let now = Instant::now();
        DiagnosticsSnapshot {
            state: format!("{:?}", self.state),
            feedback_state: format!("{:?}", self.feedback_state),
            last_state_change_secs: now.duration_since(self.last_state_change).as_secs_f64(),
            connected: self.connected,
            server_address: self.server_address.clone(),
            last_ping_secs: self
                .last_ping_received
                .map(|t| now.duration_since(t).as_secs_f64()),
            uptime_seconds: now.duration_since(self.started_at).as_secs_f64(),
            interaction_count: self.interaction_count,
            satellite_name: self.satellite_name.clone(),
            area: self.area.clone(),
            audio_device: self.audio_device.clone(),
            audio_format: self.audio_format.clone(),
            stages: self.stage_names.clone(),
        }
    }
}

/// JSON-serializable projection of [`DiagnosticsState`].
/// Built on demand per HTTP request — never stored or cached.
#[derive(Debug, Serialize)]
pub struct DiagnosticsSnapshot {
    pub state: String,
    pub feedback_state: String,
    pub last_state_change_secs: f64,
    pub connected: bool,
    pub server_address: String,
    pub last_ping_secs: Option<f64>,
    pub uptime_seconds: f64,
    pub interaction_count: u64,
    pub satellite_name: String,
    pub area: Option<String>,
    pub audio_device: String,
    pub audio_format: String,
    pub stages: Vec<String>,
}

pub type SharedDiagnostics = Arc<Mutex<DiagnosticsState>>;

/// Registry of connected SSE clients. Each client has a bounded channel sender.
pub type SseClients = Arc<Mutex<Vec<SyncSender<String>>>>;

const MAX_SSE_CLIENTS: usize = 3;

/// Push a pre-formatted SSE frame to all connected clients.
///
/// Drops frames silently if a client's buffer is full (real-time data).
/// Removes disconnected clients.
pub fn push_sse(clients: &SseClients, event_type: &str, json: &str) {
    let frame = format!("event: {}\ndata: {}\n\n", event_type, json);
    let mut clients = clients.lock().unwrap();
    clients.retain(|tx| match tx.try_send(frame.clone()) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => true, // keep client, drop frame
        Err(TrySendError::Disconnected(_)) => false, // remove dead client
    });
}

/// Start the diagnostics HTTP server on a background thread.
pub fn spawn_http_server(bind_addr: &str, diagnostics: SharedDiagnostics, sse_clients: SseClients) {
    let listener = TcpListener::bind(bind_addr)
        .unwrap_or_else(|e| panic!("Failed to bind diagnostics server on {}: {}", bind_addr, e));

    log::info!("Diagnostics HTTP server listening on {}", bind_addr);

    std::thread::Builder::new()
        .name("diagnostics-http".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let _ = stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)));
                        let _ = stream
                            .set_write_timeout(Some(std::time::Duration::from_secs(5)));
                        if let Err(e) = handle_request(&mut stream, &diagnostics, &sse_clients) {
                            log::debug!("Diagnostics HTTP error: {}", e);
                        }
                    }
                    Err(e) => {
                        log::debug!("Diagnostics HTTP accept error: {}", e);
                    }
                }
            }
        })
        .expect("failed to spawn diagnostics HTTP thread");
}

fn handle_request(
    stream: &mut std::net::TcpStream,
    diagnostics: &SharedDiagnostics,
    sse_clients: &SseClients,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    // Parse method and path from request line (e.g. "POST /vad/threshold HTTP/1.1")
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    // Consume remaining headers, capture Content-Length
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(val) = line.strip_prefix("Content-Length:") {
            content_length = val.trim().parse().unwrap_or(0);
        }
        // Case-insensitive fallback
        if let Some(val) = line.strip_prefix("content-length:") {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }

    match (method, path) {
        ("GET", "/health") => {
            let connected = diagnostics.lock().unwrap().connected;
            let (status, body) = if connected {
                ("200 OK", "healthy")
            } else {
                ("503 Service Unavailable", "disconnected")
            };
            write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            )?;
        }
        ("GET", "/") => {
            let snapshot = diagnostics.lock().unwrap().to_snapshot();
            let json = serde_json::to_string_pretty(&snapshot)?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                json.len(),
                json
            )?;
        }
        ("GET", "/rollups") => {
            let entries = &diagnostics.lock().unwrap().rollup_entries;
            let json = serde_json::to_string(entries)?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                json.len(),
                json
            )?;
        }
        ("GET", "/sessions") => {
            let sessions = diagnostics.lock().unwrap().session_tracker.sessions().clone();
            let json = serde_json::to_string(&sessions)?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                json.len(),
                json
            )?;
        }
        ("GET", "/stream") => {
            handle_sse_connection(stream, diagnostics, sse_clients)?;
            // SSE connection runs in a spawned thread; don't flush/close here
            return Ok(());
        }
        ("POST", "/vad/attack_ratio") => {
            handle_set_ratio(stream, &mut reader, diagnostics, content_length, RatioKind::Attack)?;
        }
        ("POST", "/vad/sustain_ratio") => {
            handle_set_ratio(stream, &mut reader, diagnostics, content_length, RatioKind::Sustain)?;
        }
        ("POST", "/pipeline/reset") => {
            diagnostics.lock().unwrap().pending_reset = true;
            log::info!("Pipeline reset queued via HTTP");
            let body = r#"{"reset":"queued"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
        }
        _ => {
            let body = "Not Found";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
        }
    }

    stream.flush()?;
    Ok(())
}

fn handle_sse_connection(
    stream: &mut std::net::TcpStream,
    diagnostics: &SharedDiagnostics,
    sse_clients: &SseClients,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check client limit
    let client_count = sse_clients.lock().unwrap().len();
    if client_count >= MAX_SSE_CLIENTS {
        let body = "Too many SSE clients";
        write!(
            stream,
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )?;
        stream.flush()?;
        return Ok(());
    }

    // Send SSE response headers
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n"
    )?;

    // Send initial snapshot
    let snapshot_json = {
        let d = diagnostics.lock().unwrap();
        d.to_sse_snapshot()
    };
    write!(stream, "event: snapshot\ndata: {}\n\n", snapshot_json)?;
    stream.flush()?;

    // Create channel and register client
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(256);
    sse_clients.lock().unwrap().push(tx);
    log::info!("SSE client connected ({} total)", client_count + 1);

    // Clone stream for the SSE writer thread — remove timeouts for long-lived connection
    let mut sse_stream = stream.try_clone()?;
    let _ = sse_stream.set_read_timeout(None);
    let _ = sse_stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));

    std::thread::Builder::new()
        .name("sse-client".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            loop {
                match rx.recv_timeout(std::time::Duration::from_secs(15)) {
                    Ok(frame) => {
                        if sse_stream.write_all(frame.as_bytes()).is_err() {
                            break;
                        }
                        if sse_stream.flush().is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Send keepalive comment
                        if write!(sse_stream, ": keepalive\n\n").is_err() {
                            break;
                        }
                        if sse_stream.flush().is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
            log::info!("SSE client disconnected");
        })?;

    Ok(())
}

enum RatioKind {
    Attack,
    Sustain,
}

fn handle_set_ratio(
    stream: &mut std::net::TcpStream,
    reader: &mut BufReader<std::net::TcpStream>,
    diagnostics: &SharedDiagnostics,
    content_length: usize,
    kind: RatioKind,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut body = vec![0u8; content_length.min(64)];
    std::io::Read::read_exact(reader, &mut body)?;
    let body_str = String::from_utf8_lossy(&body);

    let label = match kind {
        RatioKind::Attack => "attack_ratio",
        RatioKind::Sustain => "sustain_ratio",
    };

    let ratio: f32 = match body_str.trim().parse() {
        Ok(v) if v > 0.0 => v,
        _ => {
            let err = format!(
                r#"{{"error":"{} must be a positive number"}}"#,
                label
            );
            write!(
                stream,
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                err.len(),
                err
            )?;
            return Ok(());
        }
    };

    {
        let mut d = diagnostics.lock().unwrap();
        match kind {
            RatioKind::Attack => d.pending_attack_ratio = Some(ratio),
            RatioKind::Sustain => d.pending_sustain_ratio = Some(ratio),
        }
    }
    log::info!("{} update queued: {}", label, ratio);

    let resp = format!(r#"{{"{}":{}}}"#, label, ratio);
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp.len(),
        resp
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sse_clients() -> SseClients {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn test_config() -> Config {
        let toml = r#"
[satellite]
name = "test-sat"
area = "Office"

[server]
host = "10.0.0.1"
port = 10700

[audio]
wav_input = "test.wav"

[pipeline]
silence_timeout_ms = 2500

[pipeline.auto_energy]
attack_ratio = 3.0
sustain_ratio = 1.5
"#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn snapshot_from_fresh_state() {
        let config = test_config();
        let state = DiagnosticsState::new(&config);
        let snap = state.to_snapshot();

        assert_eq!(snap.state, "Idle");
        assert_eq!(snap.feedback_state, "Idle");
        assert!(!snap.connected);
        assert_eq!(snap.satellite_name, "test-sat");
        assert_eq!(snap.area.as_deref(), Some("Office"));
        assert_eq!(snap.server_address, "10.0.0.1:10700");
        assert_eq!(snap.interaction_count, 0);
        assert_eq!(snap.audio_device, "test.wav");
        assert_eq!(snap.audio_format, "16000Hz 16bit 1ch");
        assert!(snap.last_ping_secs.is_none());
    }

    #[test]
    fn snapshot_reflects_state_changes() {
        let config = test_config();
        let mut state = DiagnosticsState::new(&config);

        state.state = SatelliteState::Streaming;
        state.feedback_state = FeedbackState::Listening;
        state.connected = true;
        state.interaction_count = 3;

        let snap = state.to_snapshot();
        assert_eq!(snap.state, "Streaming");
        assert_eq!(snap.feedback_state, "Listening");
        assert!(snap.connected);
        assert_eq!(snap.interaction_count, 3);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let config = test_config();
        let state = DiagnosticsState::new(&config);
        let snap = state.to_snapshot();
        let json = serde_json::to_string(&snap).unwrap();

        assert!(json.contains("\"state\":\"Idle\""));
        assert!(json.contains("\"satellite_name\":\"test-sat\""));
    }

    #[test]
    fn http_health_check_integration() {
        let config = test_config();
        let diag = Arc::new(Mutex::new(DiagnosticsState::new(&config)));

        // Bind to random port
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let diag_clone = Arc::clone(&diag);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            handle_request(&mut stream, &diag_clone, &test_sse_clients()).unwrap();
        });

        // Disconnected → 503
        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        write!(stream, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 503"));
        assert!(response.contains("disconnected"));
    }

    #[test]
    fn http_root_returns_json() {
        let config = test_config();
        let diag = Arc::new(Mutex::new(DiagnosticsState::new(&config)));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let diag_clone = Arc::clone(&diag);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            handle_request(&mut stream, &diag_clone, &test_sse_clients()).unwrap();
        });

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        write!(stream, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("application/json"));
        assert!(response.contains("\"satellite_name\""));
    }

    #[test]
    fn http_unknown_path_returns_404() {
        let config = test_config();
        let diag = Arc::new(Mutex::new(DiagnosticsState::new(&config)));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let diag_clone = Arc::clone(&diag);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            handle_request(&mut stream, &diag_clone, &test_sse_clients()).unwrap();
        });

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        write!(stream, "GET /foo HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn http_health_connected_returns_200() {
        let config = test_config();
        let diag = Arc::new(Mutex::new(DiagnosticsState::new(&config)));
        diag.lock().unwrap().connected = true;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let diag_clone = Arc::clone(&diag);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            handle_request(&mut stream, &diag_clone, &test_sse_clients()).unwrap();
        });

        let mut stream = std::net::TcpStream::connect(addr).unwrap();
        write!(stream, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("healthy"));
    }
}
