use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Intent was handled. Event type: "handled"
#[derive(Debug, Clone, PartialEq)]
pub struct Handled {
    pub text: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for Handled {
    const EVENT_TYPE: &'static str = "handled";

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

/// Intent was not handled. Event type: "not-handled"
#[derive(Debug, Clone, PartialEq)]
pub struct NotHandled {
    pub text: Option<String>,
    pub context: Option<Value>,
}

impl Eventable for NotHandled {
    const EVENT_TYPE: &'static str = "not-handled";

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

/// Start streaming handled response. Event type: "handled-start"
#[derive(Debug, Clone, PartialEq)]
pub struct HandledStart {
    pub context: Option<Value>,
}

impl Eventable for HandledStart {
    const EVENT_TYPE: &'static str = "handled-start";

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

/// A chunk of handled response text. Event type: "handled-chunk"
#[derive(Debug, Clone, PartialEq)]
pub struct HandledChunk {
    pub text: String,
}

impl Eventable for HandledChunk {
    const EVENT_TYPE: &'static str = "handled-chunk";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("text".into(), Value::String(self.text));
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
        Ok(Self { text })
    }
}

/// Stop streaming handled response. Event type: "handled-stop"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandledStop;

impl Eventable for HandledStop {
    const EVENT_TYPE: &'static str = "handled-stop";

    fn into_event(self) -> Event {
        Event::new(Self::EVENT_TYPE)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        Ok(Self)
    }
}
