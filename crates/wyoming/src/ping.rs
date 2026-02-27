use serde_json::Value;

use crate::event::{ConversionError, Event, Eventable};

/// Keepalive ping from server to satellite (or vice versa).
///
/// Home Assistant sends these periodically (~2s after idle) and expects a
/// `Pong` reply within 5 seconds. Failure to reply causes HA to drop
/// the connection and reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ping {
    /// Opaque text echoed back in the Pong response.
    pub text: String,
}

impl Eventable for Ping {
    const EVENT_TYPE: &'static str = "ping";

    fn into_event(self) -> Event {
        let mut data = serde_json::Map::new();
        if !self.text.is_empty() {
            data.insert("text".into(), Value::String(self.text));
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
        let text = event
            .data
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Self { text })
    }
}

/// Keepalive pong reply — sent in response to a `Ping`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pong {
    /// Echoed text from the original Ping.
    pub text: String,
}

impl Eventable for Pong {
    const EVENT_TYPE: &'static str = "pong";

    fn into_event(self) -> Event {
        let mut data = serde_json::Map::new();
        if !self.text.is_empty() {
            data.insert("text".into(), Value::String(self.text));
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
        let text = event
            .data
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Self { text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trip() {
        let ping = Ping {
            text: "hello".into(),
        };
        let event = ping.into_event();
        assert_eq!(event.event_type, "ping");
        let recovered = Ping::from_event(event).unwrap();
        assert_eq!(recovered.text, "hello");
    }

    #[test]
    fn ping_empty_text() {
        let ping = Ping {
            text: String::new(),
        };
        let event = ping.into_event();
        let recovered = Ping::from_event(event).unwrap();
        assert_eq!(recovered.text, "");
    }

    #[test]
    fn pong_round_trip() {
        let pong = Pong {
            text: "world".into(),
        };
        let event = pong.into_event();
        assert_eq!(event.event_type, "pong");
        let recovered = Pong::from_event(event).unwrap();
        assert_eq!(recovered.text, "world");
    }

    #[test]
    fn pong_empty_text() {
        let pong = Pong {
            text: String::new(),
        };
        let event = pong.into_event();
        let recovered = Pong::from_event(event).unwrap();
        assert_eq!(recovered.text, "");
    }

    #[test]
    fn wrong_type_errors() {
        let event = Event::new("not-a-ping");
        assert!(Ping::from_event(event.clone()).is_err());
        assert!(Pong::from_event(event).is_err());
    }
}
