mod config;
mod connection;
mod diagnostics;
mod feedback;
mod hardware;
mod service;
mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::Config;
use diagnostics::{DiagnosticsState, SharedDiagnostics};
use service::SatelliteService;
use state::{transition, Action, FeedbackState, SatelliteState};

fn main() {
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

fn run(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let mut service = SatelliteService::new(config)?;

    let diagnostics: SharedDiagnostics = Arc::new(Mutex::new(DiagnosticsState::new(config)));

    if config.diagnostics.enabled {
        let bind_addr = format!("{}:{}", config.diagnostics.bind, config.diagnostics.port);
        diagnostics::spawn_http_server(&bind_addr, Arc::clone(&diagnostics));
    }

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
