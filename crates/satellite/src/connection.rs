use std::io::{BufReader, BufWriter};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use thiserror::Error;

use wyoming::event::{self, Event, Eventable, ProtocolError};
use wyoming::info::{Describe, Info, SatelliteInfo};
use wyoming::satellite::RunSatellite;

use crate::config::ServerConfig;
use crate::state::SatelliteInput;

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unexpected event during handshake: expected {expected}, got {actual}")]
    UnexpectedEvent { expected: String, actual: String },

}

/// Manages the TCP connection to the Home Assistant Wyoming server.
///
/// Holds separate reader/writer handles to the same TCP socket.
/// The reader is buffered (required by the Wyoming wire format parser).
pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl Connection {
    /// Connect to the server and perform the describe/info handshake.
    ///
    /// 1. TCP connect
    /// 2. Wait for `describe` from server
    /// 3. Reply with `info` containing our satellite metadata
    pub fn connect(
        config: &ServerConfig,
        sat_info: &SatelliteInfo,
    ) -> Result<Self, ConnectionError> {
        let addr = format!("{}:{}", config.host, config.port);
        log::info!("Connecting to {}", addr);

        let stream = TcpStream::connect(&addr)?;
        stream.set_nodelay(true)?;

        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);
        let mut conn = Self { reader, writer };

        // Handshake: server sends `describe`, we reply with `info`
        conn.handle_describe(sat_info)?;

        Ok(conn)
    }

    /// Wrap a pre-accepted TcpStream (listen mode) and handle the describe/info handshake.
    ///
    /// In listen mode, the satellite is the TCP server. HA connects to us and
    /// sends `describe`; we reply with `info` — same handshake as connect mode.
    pub fn from_stream(
        stream: TcpStream,
        sat_info: &SatelliteInfo,
    ) -> Result<Self, ConnectionError> {
        stream.set_nodelay(true)?;

        let reader = BufReader::new(stream.try_clone()?);
        let writer = BufWriter::new(stream);
        let mut conn = Self { reader, writer };

        // Same handshake: HA sends `describe`, we reply with `info`
        conn.handle_describe(sat_info)?;

        Ok(conn)
    }

    /// Handle the describe/info handshake.
    fn handle_describe(&mut self, sat_info: &SatelliteInfo) -> Result<(), ConnectionError> {
        let event = event::read_event(&mut self.reader)?;

        if event.event_type != Describe::EVENT_TYPE {
            return Err(ConnectionError::UnexpectedEvent {
                expected: Describe::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }

        log::debug!("Received describe, sending info");

        let info = Info {
            satellite: Some(sat_info.clone()),
        };
        self.send(info)?;
        Ok(())
    }

    /// Block until the server sends `run-satellite`.
    /// Any other events received while waiting are logged and discarded.
    pub fn wait_for_run(&mut self) -> Result<(), ConnectionError> {
        log::info!("Waiting for run-satellite...");
        loop {
            let event = event::read_event(&mut self.reader)?;
            if event.event_type == RunSatellite::EVENT_TYPE {
                log::info!("Received run-satellite, entering main loop");
                return Ok(());
            }
            log::debug!(
                "Ignoring event while waiting for run-satellite: {}",
                event.event_type
            );
        }
    }

    /// Send a typed event to the server.
    pub fn send<E: Eventable>(&mut self, event: E) -> Result<(), ConnectionError> {
        let raw = event.into_event();
        log::trace!("Sending: {}", raw.event_type);
        event::write_event(&mut self.writer, &raw)?;
        Ok(())
    }

    /// Block until the next event from the server.
    pub fn read_event(&mut self) -> Result<Event, ConnectionError> {
        let event = event::read_event(&mut self.reader)?;
        log::trace!("Received: {}", event.event_type);
        Ok(event)
    }

    /// Non-blocking check for a server event.
    ///
    /// Uses poll(2) to check socket readability without consuming data,
    /// then checks BufReader's internal buffer. Only calls read_event
    /// when data is known to be available, avoiding the BufReader + timeout
    /// inconsistency where a partial event could corrupt the stream.
    pub fn try_read_event(&mut self) -> Result<Option<Event>, ConnectionError> {
        // If BufReader has data in its internal buffer, an event may be ready.
        // Otherwise, poll the raw fd to check for new data without consuming anything.
        if self.reader.buffer().is_empty() && !self.is_socket_readable()? {
            return Ok(None);
        }

        // Data is available — read the complete event.
        // Wyoming events are small (< 1KB) and arrive as complete TCP segments,
        // so this read should complete without blocking.
        let event = event::read_event(&mut self.reader)?;
        log::trace!("Received (non-blocking): {}", event.event_type);
        Ok(Some(event))
    }

    /// Check if the underlying socket has data available to read,
    /// using poll(2) with zero timeout. Does not interact with BufReader.
    fn is_socket_readable(&self) -> Result<bool, std::io::Error> {
        let fd = self.reader.get_ref().as_raw_fd();
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ret = unsafe { libc::poll(&mut pollfd, 1, 0) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(ret > 0 && (pollfd.revents & libc::POLLIN) != 0)
        }
    }
}

/// Attempt to connect with exponential backoff.
/// Retries: 1s, 2s, 4s, 8s, 16s, 30s, 30s, ...
pub fn connect_with_retry(
    config: &ServerConfig,
    sat_info: &SatelliteInfo,
    max_attempts: Option<u32>,
) -> Result<Connection, ConnectionError> {
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        match Connection::connect(config, sat_info) {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                if let Some(max) = max_attempts {
                    if attempt >= max {
                        log::error!("Failed to connect after {} attempts: {}", attempt, e);
                        return Err(e);
                    }
                }
                log::warn!(
                    "Connection attempt {} failed: {}. Retrying in {:?}...",
                    attempt,
                    e,
                    delay
                );
                std::thread::sleep(delay);
                delay = (delay * 2).min(max_delay);
            }
        }
    }
}

/// Map a raw server event to a SatelliteInput for the state machine.
///
/// Returns None for events the state machine doesn't care about
/// (e.g. audio-chunk during Responding is handled directly by the main loop).
pub fn map_server_event(event: &Event) -> Option<SatelliteInput> {
    match event.event_type.as_str() {
        "run-satellite" => Some(SatelliteInput::ServerRunSatellite),
        "pause-satellite" => Some(SatelliteInput::ServerPauseSatellite),
        "detection" => Some(SatelliteInput::ServerDetection),
        "voice-started" => Some(SatelliteInput::ServerVoiceStarted),
        "audio-start" => Some(SatelliteInput::ServerTtsStart),
        "audio-stop" => Some(SatelliteInput::ServerTtsStop),
        "voice-stopped" => Some(SatelliteInput::ServerVoiceStopped),
        other => {
            log::debug!("Unmapped server event: {}", other);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_all_known_server_events() {
        let cases = vec![
            ("run-satellite", SatelliteInput::ServerRunSatellite),
            ("pause-satellite", SatelliteInput::ServerPauseSatellite),
            ("detection", SatelliteInput::ServerDetection),
            ("voice-started", SatelliteInput::ServerVoiceStarted),
            ("audio-start", SatelliteInput::ServerTtsStart),
            ("audio-stop", SatelliteInput::ServerTtsStop),
            ("voice-stopped", SatelliteInput::ServerVoiceStopped),
        ];

        for (event_type, expected) in cases {
            let event = Event::new(event_type);
            assert_eq!(
                map_server_event(&event),
                Some(expected),
                "Failed for event type: {}",
                event_type,
            );
        }
    }

    #[test]
    fn map_unknown_event_returns_none() {
        let event = Event::new("some-unknown-event");
        assert_eq!(map_server_event(&event), None);
    }

    #[test]
    fn map_audio_chunk_returns_none() {
        // audio-chunk during Responding is handled directly, not via state machine
        let event = Event::new("audio-chunk");
        assert_eq!(map_server_event(&event), None);
    }
}
