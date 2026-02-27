use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

use crate::config::Config;
use crate::state::{FeedbackState, SatelliteState};

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
    pub vad_current_energy: Option<u16>,
    pub interaction_count: u64,

    // ── Static (set once at startup) ────────────────────────────────────
    started_at: Instant,
    satellite_name: String,
    area: Option<String>,
    server_address: String,
    audio_device: String,
    audio_format: String,
    vad_mode: String,
    vad_threshold: Option<u16>,
}

impl DiagnosticsState {
    pub fn new(config: &Config) -> Self {
        let now = Instant::now();
        let (vad_mode, vad_threshold) = match &config.vad {
            crate::config::VadConfig::AlwaysOn { .. } => ("always_on".into(), None),
            crate::config::VadConfig::Gpio { pin, .. } => (format!("gpio(pin={})", pin), None),
            crate::config::VadConfig::Energy { threshold, .. } => {
                ("energy".into(), Some(*threshold))
            }
        };
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
            vad_current_energy: None,
            interaction_count: 0,
            started_at: now,
            satellite_name: config.satellite.name.clone(),
            area: config.satellite.area.clone(),
            server_address: format!("{}:{}", config.server.host, config.server.port),
            audio_device,
            audio_format,
            vad_mode,
            vad_threshold,
        }
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
            vad_mode: self.vad_mode.clone(),
            vad_threshold: self.vad_threshold,
            vad_current_energy: self.vad_current_energy,
            vad_triggered: match (self.vad_current_energy, self.vad_threshold) {
                (Some(energy), Some(threshold)) => energy >= threshold,
                _ => false,
            },
            uptime_seconds: now.duration_since(self.started_at).as_secs_f64(),
            interaction_count: self.interaction_count,
            satellite_name: self.satellite_name.clone(),
            area: self.area.clone(),
            audio_device: self.audio_device.clone(),
            audio_format: self.audio_format.clone(),
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
    pub vad_mode: String,
    pub vad_threshold: Option<u16>,
    pub vad_current_energy: Option<u16>,
    pub vad_triggered: bool,
    pub uptime_seconds: f64,
    pub interaction_count: u64,
    pub satellite_name: String,
    pub area: Option<String>,
    pub audio_device: String,
    pub audio_format: String,
}

pub type SharedDiagnostics = Arc<Mutex<DiagnosticsState>>;

/// Start the diagnostics HTTP server on a background thread.
pub fn spawn_http_server(bind_addr: &str, diagnostics: SharedDiagnostics) {
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
                        if let Err(e) = handle_request(&mut stream, &diagnostics) {
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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    // Consume remaining headers
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");

    match path {
        "/health" => {
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
        "/" => {
            let snapshot = diagnostics.lock().unwrap().to_snapshot();
            let json = serde_json::to_string_pretty(&snapshot)?;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                json.len(),
                json
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

#[cfg(test)]
mod tests {
    use super::*;

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

[vad]
mode = "energy"
threshold = 1000
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
        assert_eq!(snap.vad_mode, "energy");
        assert_eq!(snap.vad_threshold, Some(1000));
        assert_eq!(snap.vad_current_energy, None);
        assert!(!snap.vad_triggered);
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
        state.vad_current_energy = Some(1500);
        state.interaction_count = 3;

        let snap = state.to_snapshot();
        assert_eq!(snap.state, "Streaming");
        assert_eq!(snap.feedback_state, "Listening");
        assert!(snap.connected);
        assert_eq!(snap.vad_current_energy, Some(1500));
        assert!(snap.vad_triggered); // 1500 >= 1000
        assert_eq!(snap.interaction_count, 3);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let config = test_config();
        let state = DiagnosticsState::new(&config);
        let snap = state.to_snapshot();
        let json = serde_json::to_string(&snap).unwrap();

        // Verify key fields are present
        assert!(json.contains("\"state\":\"Idle\""));
        assert!(json.contains("\"satellite_name\":\"test-sat\""));
        assert!(json.contains("\"vad_threshold\":1000"));
    }

    #[test]
    fn vad_triggered_only_when_energy_exceeds_threshold() {
        let config = test_config();
        let mut state = DiagnosticsState::new(&config);

        // Below threshold
        state.vad_current_energy = Some(500);
        assert!(!state.to_snapshot().vad_triggered);

        // At threshold
        state.vad_current_energy = Some(1000);
        assert!(state.to_snapshot().vad_triggered);

        // Above threshold
        state.vad_current_energy = Some(2000);
        assert!(state.to_snapshot().vad_triggered);

        // No energy reading
        state.vad_current_energy = None;
        assert!(!state.to_snapshot().vad_triggered);
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
            handle_request(&mut stream, &diag_clone).unwrap();
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
            handle_request(&mut stream, &diag_clone).unwrap();
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
            handle_request(&mut stream, &diag_clone).unwrap();
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
            handle_request(&mut stream, &diag_clone).unwrap();
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
