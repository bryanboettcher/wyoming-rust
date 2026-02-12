use serde_json::{Map, Value};
use std::io::{BufRead, Write};
use thiserror::Error;

/// Protocol version we advertise in outgoing events.
pub const WYOMING_VERSION: &str = "1.5.2";

/// A raw Wyoming protocol event as it appears on the wire.
///
/// This is the untyped representation — the header JSON line has been parsed,
/// but the `data` and `payload` fields are still generic. Domain-specific
/// types (AudioChunk, Detection, etc.) convert to/from this via the
/// `Eventable` trait.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub event_type: String,
    pub data: Map<String, Value>,
    pub payload: Option<Vec<u8>>,
}

impl Event {
    pub fn new(event_type: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            data: Map::new(),
            payload: None,
        }
    }

    pub fn with_data(mut self, data: Map<String, Value>) -> Self {
        self.data = data;
        self
    }

    pub fn with_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// Errors during protocol read/write.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid header: {0}")]
    InvalidHeader(String),

    #[error("connection closed")]
    ConnectionClosed,
}

/// Trait for converting between typed domain structs and raw Events.
///
/// C# analogy: interface IEventable<T> {
///     static string EventType { get; }
///     Event ToEvent();
///     static T FromEvent(Event e);
/// }
pub trait Eventable: Sized {
    /// The wire type string for this event (e.g. "audio-chunk", "detection").
    const EVENT_TYPE: &'static str;

    /// Convert this typed struct into a raw Event for serialization.
    fn into_event(self) -> Event;

    /// Try to convert a raw Event into this typed struct.
    fn from_event(event: Event) -> Result<Self, ConversionError>;
}

/// Errors when converting between Event and typed domain structs.
#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("wrong event type: expected {expected}, got {actual}")]
    WrongType { expected: String, actual: String },

    #[error("missing field: {0}")]
    MissingField(String),

    #[error("invalid field value: {0}")]
    InvalidValue(String),
}

/// Read one Wyoming event from a buffered reader.
///
/// Wire format:
///   Line 1: JSON header {"type":"...", "data_length":N, "payload_length":M, "version":"..."}\n
///   Next N bytes: JSON data section (UTF-8, no trailing newline)
///   Next M bytes: raw binary payload
///
/// Blocks until a complete event is available or the connection closes.
pub fn read_event(reader: &mut impl BufRead) -> Result<Event, ProtocolError> {
    // Read the header line
    let mut header_line = String::new();
    let bytes_read = reader.read_line(&mut header_line)?;
    if bytes_read == 0 {
        return Err(ProtocolError::ConnectionClosed);
    }

    // Parse header JSON
    let header: Map<String, Value> = serde_json::from_str(header_line.trim())?;

    let event_type = header
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProtocolError::InvalidHeader("missing 'type' field".into()))?
        .to_string();

    let data_length = header
        .get("data_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let payload_length = header
        .get("payload_length")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    // Read data section
    let data: Map<String, Value> = if data_length > 0 {
        let mut data_buf = vec![0u8; data_length];
        reader.read_exact(&mut data_buf)?;
        serde_json::from_slice(&data_buf)?
    } else {
        Map::new()
    };

    // Read payload section
    let payload = if payload_length > 0 {
        let mut payload_buf = vec![0u8; payload_length];
        reader.read_exact(&mut payload_buf)?;
        Some(payload_buf)
    } else {
        None
    };

    Ok(Event {
        event_type,
        data,
        payload,
    })
}

/// Write one Wyoming event to a writer.
///
/// Produces the three-part wire format: header line + data section + payload section.
pub fn write_event(writer: &mut impl Write, event: &Event) -> Result<(), ProtocolError> {
    // Build header
    let mut header = Map::new();
    header.insert("type".into(), Value::String(event.event_type.clone()));
    header.insert(
        "version".into(),
        Value::String(WYOMING_VERSION.to_string()),
    );

    // Serialize data section to get its byte length
    let data_bytes = if !event.data.is_empty() {
        let bytes = serde_json::to_vec(&event.data)?;
        header.insert(
            "data_length".into(),
            Value::Number(serde_json::Number::from(bytes.len())),
        );
        Some(bytes)
    } else {
        None
    };

    // Payload length
    if let Some(ref payload) = event.payload {
        header.insert(
            "payload_length".into(),
            Value::Number(serde_json::Number::from(payload.len())),
        );
    }

    // Write header line + newline
    let header_json = serde_json::to_string(&Value::Object(header))?;
    writer.write_all(header_json.as_bytes())?;
    writer.write_all(b"\n")?;

    // Write data section (no trailing newline)
    if let Some(data_bytes) = data_bytes {
        writer.write_all(&data_bytes)?;
    }

    // Write payload section
    if let Some(ref payload) = event.payload {
        writer.write_all(payload)?;
    }

    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests;
