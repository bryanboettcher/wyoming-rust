use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

#[derive(Debug, Clone, PartialEq)]
pub struct TimerStarted {
    pub id: String,
    pub total_seconds: f64,
    pub name: Option<String>,
}

impl Eventable for TimerStarted {
    const EVENT_TYPE: &'static str = "timer-started";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        data.insert("total_seconds".into(), serde_json::json!(self.total_seconds));
        if let Some(name) = self.name {
            data.insert("name".into(), Value::String(name));
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
        let id = event.data.get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        let total_seconds = event.data.get("total_seconds")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ConversionError::MissingField("total_seconds".into()))?;
        let name = event.data.get("name").and_then(|v| v.as_str().map(String::from));
        Ok(Self { id, total_seconds, name })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerUpdated {
    pub id: String,
    pub is_active: bool,
    pub total_seconds: f64,
}

impl Eventable for TimerUpdated {
    const EVENT_TYPE: &'static str = "timer-updated";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        data.insert("is_active".into(), Value::Bool(self.is_active));
        data.insert("total_seconds".into(), serde_json::json!(self.total_seconds));
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let id = event.data.get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        let is_active = event.data.get("is_active")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| ConversionError::MissingField("is_active".into()))?;
        let total_seconds = event.data.get("total_seconds")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ConversionError::MissingField("total_seconds".into()))?;
        Ok(Self { id, is_active, total_seconds })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerCancelled {
    pub id: String,
}

impl Eventable for TimerCancelled {
    const EVENT_TYPE: &'static str = "timer-cancelled";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let id = event.data.get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        Ok(Self { id })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimerFinished {
    pub id: String,
}

impl Eventable for TimerFinished {
    const EVENT_TYPE: &'static str = "timer-finished";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let id = event.data.get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        Ok(Self { id })
    }
}
