mod config;
mod connection;
mod hardware;
mod service;
mod state;

use std::path::PathBuf;
use std::time::Duration;

use config::Config;
use service::SatelliteService;
use state::{transition, Action, LedState, SatelliteState};

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

    // Outer loop: connection lifecycle
    loop {
        match service.establish_connection() {
            Ok(()) => {
                log::info!("Session starting");
                run_session(&mut service);
                log::info!("Session ended, waiting for next connection");
            }
            Err(e) => {
                log::error!("Connection failed: {}", e);
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

fn run_session(service: &mut SatelliteService) {
    let mut state = SatelliteState::Idle;
    service.execute(&Action::SetLed(LedState::DimWhite)).ok();

    log::info!("Entering main loop (state: Idle)");

    loop {
        let input = match service.next_input(&state) {
            Ok(Some(input)) => input,
            Ok(None) => continue,
            Err(e) => {
                log::error!("I/O error: {}", e);
                return;
            }
        };

        log::debug!("Input: {:?} (state: {:?})", input, state);

        let (new_state, actions) = transition(&state, &input);

        if new_state != state {
            log::info!("State: {:?} -> {:?}", state, new_state);
        }

        for action in &actions {
            if matches!(action, Action::Reconnect) {
                // Exit session; outer loop handles re-connection
                return;
            }
            if let Err(e) = service.execute(action) {
                log::error!("Action error: {}", e);
                return;
            }
        }

        state = new_state;
    }
}
