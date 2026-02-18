use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// A named entity extracted from an intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Request to recognize an intent from text. Event type: "recognize"
#[derive(Debug, Clone, PartialEq)]
pub struct Recognize {
    pub text: String,
    pub context: Option<Value>,
}

impl Eventable for Recognize {
    const EVENT_TYPE: &'static str = "recognize";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("text".into(), Value::String(self.text));
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
            context: event.data.get("context").cloned(),
        })
    }
}

/// A recognized intent. Event type: "intent"
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub name: String,
    pub entities: Vec<Entity>,
    pub text: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for Intent {
    const EVENT_TYPE: &'static str = "intent";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("name".into(), Value::String(self.name));
        if !self.entities.is_empty() {
            let entities_val = serde_json::to_value(&self.entities).unwrap_or(Value::Array(vec![]));
            data.insert("entities".into(), entities_val);
        }
        if let Some(text) = self.text {
            data.insert("text".into(), Value::String(text));
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
        let name = event
            .data
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConversionError::MissingField("name".into()))?
            .to_string();
        let entities: Vec<Entity> = event
            .data
            .get("entities")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let text = event
            .data
            .get("text")
            .and_then(|v| v.as_str().map(String::from));
        let context = event.data.get("context").cloned();
        Ok(Self {
            name,
            entities,
            text,
            context,
        })
    }
}

/// Intent was not recognized. Event type: "not-recognized"
#[derive(Debug, Clone, PartialEq)]
pub struct NotRecognized {
    pub text: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for NotRecognized {
    const EVENT_TYPE: &'static str = "not-recognized";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(text) = self.text {
            data.insert("text".into(), Value::String(text));
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
            text: event
                .data
                .get("text")
                .and_then(|v| v.as_str().map(String::from)),
            context: event.data.get("context").cloned(),
        })
    }
}
