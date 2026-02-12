use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Shared audio format metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub rate: u32,
    pub width: u16,
    pub channels: u16,
}

impl AudioFormat {
    /// Standard Wyoming audio format: 16kHz, 16-bit, mono.
    pub const WYOMING_DEFAULT: Self = Self {
        rate: 16000,
        width: 2,
        channels: 1,
    };
}

/// Signals the start of an audio stream with format metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStart {
    pub format: AudioFormat,
    pub timestamp: Option<u64>,
}

impl Eventable for AudioStart {
    const EVENT_TYPE: &'static str = "audio-start";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("rate".into(), Value::Number(self.format.rate.into()));
        data.insert("width".into(), Value::Number(self.format.width.into()));
        data.insert(
            "channels".into(),
            Value::Number(self.format.channels.into()),
        );
        if let Some(ts) = self.timestamp {
            data.insert("timestamp".into(), Value::Number(ts.into()));
        }
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let rate = event
            .data
            .get("rate")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("rate".into()))? as u32;
        let width = event
            .data
            .get("width")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("width".into()))? as u16;
        let channels = event
            .data
            .get("channels")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("channels".into()))? as u16;
        let timestamp = event.data.get("timestamp").and_then(|v| v.as_u64());

        Ok(Self {
            format: AudioFormat {
                rate,
                width,
                channels,
            },
            timestamp,
        })
    }
}

/// A chunk of raw PCM audio data.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub format: AudioFormat,
    pub audio: Vec<u8>,
    pub timestamp: Option<u64>,
}

impl AudioChunk {
    /// Create an AudioChunk from raw PCM bytes with the default Wyoming format.
    pub fn from_pcm(audio: Vec<u8>) -> Self {
        Self {
            format: AudioFormat::WYOMING_DEFAULT,
            audio,
            timestamp: None,
        }
    }
}

impl Eventable for AudioChunk {
    const EVENT_TYPE: &'static str = "audio-chunk";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("rate".into(), Value::Number(self.format.rate.into()));
        data.insert("width".into(), Value::Number(self.format.width.into()));
        data.insert(
            "channels".into(),
            Value::Number(self.format.channels.into()),
        );
        if let Some(ts) = self.timestamp {
            data.insert("timestamp".into(), Value::Number(ts.into()));
        }
        Event::new(Self::EVENT_TYPE)
            .with_data(data)
            .with_payload(self.audio)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let rate = event
            .data
            .get("rate")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("rate".into()))? as u32;
        let width = event
            .data
            .get("width")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("width".into()))? as u16;
        let channels = event
            .data
            .get("channels")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("channels".into()))? as u16;
        let timestamp = event.data.get("timestamp").and_then(|v| v.as_u64());
        let audio = event.payload.unwrap_or_default();

        Ok(Self {
            format: AudioFormat {
                rate,
                width,
                channels,
            },
            audio,
            timestamp,
        })
    }
}

/// Signals the end of an audio stream.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStop {
    pub timestamp: Option<u64>,
}

impl Eventable for AudioStop {
    const EVENT_TYPE: &'static str = "audio-stop";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(ts) = self.timestamp {
            data.insert("timestamp".into(), Value::Number(ts.into()));
        }
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let timestamp = event.data.get("timestamp").and_then(|v| v.as_u64());
        Ok(Self { timestamp })
    }
}
