use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Request to transcribe audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcribe {
    pub name: Option<String>,
    pub language: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for Transcribe {
    const EVENT_TYPE: &'static str = "transcribe";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(name) = self.name {
            data.insert("name".into(), Value::String(name));
        }
        if let Some(language) = self.language {
            data.insert("language".into(), Value::String(language));
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
            language: event.data.get("language").and_then(|v| v.as_str().map(String::from)),
            context: event.data.get("context").cloned(),
        })
    }
}

/// Speech-to-text result.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for Transcript {
    const EVENT_TYPE: &'static str = "transcript";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("text".into(), Value::String(self.text));
        if let Some(language) = self.language {
            data.insert("language".into(), Value::String(language));
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
        let text = event
            .data
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConversionError::MissingField("text".into()))?
            .to_string();
        Ok(Self {
            text,
            language: event.data.get("language").and_then(|v| v.as_str().map(String::from)),
            context: event.data.get("context").cloned(),
        })
    }
}
