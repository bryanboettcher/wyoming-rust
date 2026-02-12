mod config;
mod connection;
mod hardware;
mod state;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use wyoming::audio::{AudioChunk, AudioFormat, AudioStart, AudioStop};
use wyoming::info::SatelliteInfo;
use wyoming::satellite::{Played, StreamingStarted, StreamingStopped};

use config::Config;
use connection::{connect_with_retry, map_server_event, Connection, ConnectionError};
use hardware::{AudioError, AudioSink, AudioSource, GpioInput, LedOutput};
use state::{transition, Action, LedState, SatelliteInput, SatelliteState};

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
        "Wyoming satellite '{}' starting (server {}:{})",
        config.satellite.name,
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
    let sat_info = SatelliteInfo {
        name: config.satellite.name.clone(),
        area: config.satellite.area.clone(),
        has_mic: true,
        has_snd: config.audio.wav_output.is_some()
            || config.audio.device.is_some(),
    };

    // Create hardware abstractions based on config
    let mut mic = create_audio_source(config)?;
    let mut spk = create_audio_sink(config)?;
    let gpio = create_gpio(config);
    let mut led = create_led(config);

    // Connect to server (with retry)
    let mut conn = connect_with_retry(&config.server, &sat_info, None)?;

    // Wait for server to send run-satellite
    conn.wait_for_run()?;

    let audio_format = config.audio.audio_format();
    let silence_timeout = Duration::from_millis(config.gpio.silence_timeout_ms);

    // Main state machine loop
    let mut sat_state = SatelliteState::Idle;
    let mut last_gpio_high = Instant::now();

    led.set(LedState::DimWhite);
    log::info!("Entering main loop (state: Idle)");

    loop {
        // Gather the next input based on current state
        let input = match gather_input(
            &sat_state,
            &mut conn,
            &mut *mic,
            &*gpio,
            &audio_format,
            silence_timeout,
            &mut last_gpio_high,
            &mut *spk,
        ) {
            Ok(Some(input)) => input,
            Ok(None) => continue, // No event, loop again
            Err(e) => {
                log::error!("Error gathering input: {}", e);
                SatelliteInput::Disconnected
            }
        };

        log::debug!("Input: {:?} (state: {:?})", input, sat_state);

        // Pure state transition
        let (new_state, actions) = transition(&sat_state, &input);

        if new_state != sat_state {
            log::info!("State: {:?} -> {:?}", sat_state, new_state);
        }

        // Execute side effects
        for action in &actions {
            if let Err(e) = execute_action(
                action,
                &mut conn,
                &mut *mic,
                &mut *spk,
                &mut *led,
                &audio_format,
                config,
                &sat_info,
            ) {
                log::error!("Error executing action {:?}: {}", action, e);
            }
        }

        sat_state = new_state;
    }
}

/// Gather the next state machine input based on current state.
///
/// Returns Ok(None) when there's no event to process (continue looping).
fn gather_input(
    state: &SatelliteState,
    conn: &mut Connection,
    mic: &mut dyn AudioSource,
    gpio: &dyn GpioInput,
    audio_format: &AudioFormat,
    silence_timeout: Duration,
    last_gpio_high: &mut Instant,
    spk: &mut dyn AudioSink,
) -> Result<Option<SatelliteInput>, Box<dyn std::error::Error>> {
    match state {
        // IDLE: poll GPIO and server socket
        SatelliteState::Idle => {
            // Check for GPIO trigger
            if gpio.wait_for_high(Duration::from_millis(100)) {
                *last_gpio_high = Instant::now();
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
                Err(ConnectionError::Protocol(wyoming::event::ProtocolError::ConnectionClosed)) => {
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
        SatelliteState::Streaming | SatelliteState::Triggered | SatelliteState::Processing => {
            // 1. Read one mic frame (~20ms block)
            let frame = match mic.read_frame() {
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
                        Err(ConnectionError::Protocol(
                            wyoming::event::ProtocolError::ConnectionClosed,
                        )) => return Ok(Some(SatelliteInput::Disconnected)),
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
                    format: *audio_format,
                    audio: frame,
                    timestamp: None,
                };
                conn.send(chunk)?;
            }

            // 3. Check GPIO for silence timeout
            if gpio.is_high() {
                *last_gpio_high = Instant::now();
            } else if last_gpio_high.elapsed() > silence_timeout {
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
                Err(ConnectionError::Protocol(wyoming::event::ProtocolError::ConnectionClosed)) => {
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
                Err(ConnectionError::Protocol(wyoming::event::ProtocolError::ConnectionClosed)) => {
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
                    if let Err(e) = spk.write_frame(payload) {
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

/// Execute a single action produced by the state machine.
fn execute_action(
    action: &Action,
    conn: &mut Connection,
    mic: &mut dyn AudioSource,
    spk: &mut dyn AudioSink,
    led: &mut dyn LedOutput,
    audio_format: &AudioFormat,
    config: &Config,
    sat_info: &SatelliteInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        Action::StartCapture => {
            log::debug!("Action: StartCapture");
            mic.start()?;
        }
        Action::StopCapture => {
            log::debug!("Action: StopCapture");
            mic.stop()?;
        }
        Action::SendAudioStart => {
            log::debug!("Action: SendAudioStart");
            conn.send(AudioStart {
                format: *audio_format,
                timestamp: None,
            })?;
        }
        Action::SendAudioStop => {
            log::debug!("Action: SendAudioStop");
            conn.send(AudioStop { timestamp: None })?;
        }
        Action::SendStreamingStarted => {
            log::debug!("Action: SendStreamingStarted");
            conn.send(StreamingStarted)?;
        }
        Action::SendStreamingStopped => {
            log::debug!("Action: SendStreamingStopped");
            conn.send(StreamingStopped)?;
        }
        Action::StartPlayback => {
            log::debug!("Action: StartPlayback");
            spk.start()?;
        }
        Action::StopPlayback => {
            log::debug!("Action: StopPlayback");
            spk.stop()?;
        }
        Action::SendPlayed => {
            log::debug!("Action: SendPlayed");
            conn.send(Played)?;
        }
        Action::SetLed(state) => {
            log::debug!("Action: SetLed({:?})", state);
            led.set(*state);
        }
        Action::Reconnect => {
            log::info!("Action: Reconnect");
            match connect_with_retry(&config.server, sat_info, Some(10)) {
                Ok(new_conn) => {
                    *conn = new_conn;
                    conn.wait_for_run()?;
                    log::info!("Reconnected successfully");
                }
                Err(e) => {
                    log::error!("Failed to reconnect: {}", e);
                    return Err(e.into());
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Hardware Factory Functions
// ============================================================================

fn create_audio_source(
    config: &Config,
) -> Result<Box<dyn AudioSource>, Box<dyn std::error::Error>> {
    if let Some(ref wav_path) = config.audio.wav_input {
        let source = hardware::WavFileSource::open(
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
        Ok(Box::new(hardware::WavFileSink::new(
            wav_path,
            config.audio.rate,
            config.audio.channels,
        )))
    } else {
        Ok(Box::new(hardware::NullSink))
    }
}

fn create_gpio(config: &Config) -> Box<dyn GpioInput> {
    if config.gpio.auto_trigger || config.gpio.vad_pin.is_none() {
        Box::new(hardware::AutoGpio::new())
    } else {
        log::warn!("Real GPIO not implemented, falling back to auto-trigger");
        Box::new(hardware::AutoGpio::new())
    }
}

fn create_led(config: &Config) -> Box<dyn LedOutput> {
    if config.led.method == "none" || config.led.pin.is_none() {
        Box::new(hardware::StubLed::new())
    } else {
        log::warn!("Real LED not implemented, falling back to stub");
        Box::new(hardware::StubLed::new())
    }
}
