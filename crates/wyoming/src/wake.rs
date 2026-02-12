use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Request to start wake word detection.
#[derive(Debug, Clone, PartialEq)]
pub struct Detect {
    pub names: Option<Vec<String>>,
    pub context: Option<Value>,
}

impl Eventable for Detect {
    const EVENT_TYPE: &'static str = "detect";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(names) = self.names {
            data.insert("names".into(), Value::Array(names.into_iter().map(Value::String).collect()));
        }
        if let Some(context) = self.context {
            data.insert("context".into(), context);
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
        let names = event.data.get("names").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            })
        });
        let context = event.data.get("context").cloned();
        Ok(Self { names, context })
    }
}

/// Wake word was detected by the server.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub name: Option<String>,
    pub timestamp: Option<u64>,
    pub speaker: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for Detection {
    const EVENT_TYPE: &'static str = "detection";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(name) = self.name {
            data.insert("name".into(), Value::String(name));
        }
        if let Some(ts) = self.timestamp {
            data.insert("timestamp".into(), Value::Number(ts.into()));
        }
        if let Some(speaker) = self.speaker {
            data.insert("speaker".into(), Value::String(speaker));
        }
        if let Some(context) = self.context {
            data.insert("context".into(), context);
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
        Ok(Self {
            name: event.data.get("name").and_then(|v| v.as_str().map(String::from)),
            timestamp: event.data.get("timestamp").and_then(|v| v.as_u64()),
            speaker: event.data.get("speaker").and_then(|v| v.as_str().map(String::from)),
            context: event.data.get("context").cloned(),
        })
    }
}

/// Wake word was not detected before the audio stream ended.
#[derive(Debug, Clone, PartialEq)]
pub struct NotDetected {
    pub context: Option<Value>,
}

impl Eventable for NotDetected {
    const EVENT_TYPE: &'static str = "not-detected";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(context) = self.context {
            data.insert("context".into(), context);
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
        Ok(Self {
            context: event.data.get("context").cloned(),
        })
    }
}
