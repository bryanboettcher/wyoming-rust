mod config;
mod connection;
mod diagnostics;
mod feedback;
mod hardware;
mod pipeline;
mod service;
mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::Config;
use diagnostics::{DiagnosticsState, SharedDiagnostics, SseClients};
use service::SatelliteService;
use state::{transition, Action, FeedbackState, SatelliteState};

fn main() {
    // Health check mode: connect to diagnostics endpoint and exit 0/1.
    // Runs before logger init to avoid noisy output in Docker health checks.
    if std::env::args().any(|a| a == "--health-check") {
        std::process::exit(run_health_check());
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let config_path = parse_args();
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to load config from {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };

    log::info!(
        "Wyoming satellite '{}' starting (mode: {}, {}:{})",
        config.satellite.name,
        config.server.mode,
        config.server.host,
        config.server.port
    );

    if let Err(e) = run(&config) {
        log::error!("Satellite exited with error: {}", e);
        std::process::exit(1);
    }
}

fn parse_args() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        PathBuf::from("satellite.toml")
    } else {
        PathBuf::from(&args[1])
    }
}

/// Minimal HTTP health check using raw TCP — no shell, no external binaries.
/// Reads the diagnostics port from WYOMING_HEALTH_PORT env (default 8585).
fn run_health_check() -> i32 {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let port = std::env::var("WYOMING_HEALTH_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(8585);

    let mut stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(_) => return 1,
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .ok();

    if stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return 1;
    }

    let mut buf = [0u8; 32];
    match stream.read(&mut buf) {
        Ok(n) if n >= 12 => {
            // Check for "HTTP/1.x 200"
            if buf.starts_with(b"HTTP/1.") && &buf[9..12] == b"200" {
                0
            } else {
                1
            }
        }
        _ => 1,
    }
}

fn run(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut service = SatelliteService::new(config)?;

    let diagnostics: SharedDiagnostics = Arc::new(Mutex::new(DiagnosticsState::new(config)));
    let sse_clients: SseClients = Arc::new(Mutex::new(Vec::new()));

    if config.diagnostics.enabled {
        let bind_addr = format!("{}:{}", config.diagnostics.bind, config.diagnostics.port);
        diagnostics::spawn_http_server(&bind_addr, Arc::clone(&diagnostics), Arc::clone(&sse_clients));
    }

    service.set_sse_clients(Arc::clone(&sse_clients));

    // Outer loop: connection lifecycle
    loop {
        match service.establish_connection() {
            Ok(()) => {
                log::info!("Session starting");
                run_session(&mut service, &diagnostics);
                diagnostics.lock().unwrap().connected = false;
                log::info!("Session ended, waiting for next connection");
            }
            Err(e) => {
                log::error!("Connection failed: {}", e);
                diagnostics.lock().unwrap().connected = false;
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn run_session(service: &mut SatelliteService, diagnostics: &SharedDiagnostics) {
    let mut state = SatelliteState::Idle;
    let mut feedback_state = FeedbackState::Idle;
    service.execute(&Action::SetFeedback(FeedbackState::Idle)).ok();

    // Mark connected + initial state
    {
        let mut d = diagnostics.lock().unwrap();
        d.connected = true;
        d.last_state_change = Instant::now();
    }

    log::info!("Entering main loop (state: Idle)");

    loop {
        let input = match service.next_input(&state) {
            Ok(Some(input)) => input,
            Ok(None) => {
                service.update_diagnostics(diagnostics, &state, &feedback_state);
                continue;
            }
            Err(e) => {
                log::error!("I/O error: {}", e);
                return;
            }
        };

        log::debug!("Input: {:?} (state: {:?})", input, state);

        let (new_state, actions) = transition(&state, &input);

        if new_state != state {
            log::info!("State: {:?} -> {:?}", state, new_state);
            diagnostics.lock().unwrap().last_state_change = Instant::now();

            // Track completed interactions (reached Processing/Responding, now returning to Idle)
            if new_state == SatelliteState::Idle
                && matches!(state, SatelliteState::Processing | SatelliteState::Responding)
            {
                diagnostics.lock().unwrap().interaction_count += 1;
            }
        }

        for action in &actions {
            if matches!(action, Action::Reconnect) {
                return;
            }
            if let Action::SetFeedback(fb) = action {
                feedback_state = *fb;
            }
            if let Err(e) = service.execute(action) {
                log::error!("Action error: {}", e);
                return;
            }
        }

        state = new_state;
        service.update_diagnostics(diagnostics, &state, &feedback_state);
    }
}
