use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerStarted {
    pub id: String,
    pub total_seconds: u64,
    pub name: Option<String>,
    pub start_hours: Option<u32>,
    pub start_minutes: Option<u32>,
    pub start_seconds: Option<u32>,
}

impl Eventable for TimerStarted {
    const EVENT_TYPE: &'static str = "timer-started";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        data.insert(
            "total_seconds".into(),
            Value::Number(serde_json::Number::from(self.total_seconds)),
        );
        if let Some(name) = self.name {
            data.insert("name".into(), Value::String(name));
        }
        if let Some(h) = self.start_hours {
            data.insert(
                "start_hours".into(),
                Value::Number(serde_json::Number::from(h)),
            );
        }
        if let Some(m) = self.start_minutes {
            data.insert(
                "start_minutes".into(),
                Value::Number(serde_json::Number::from(m)),
            );
        }
        if let Some(s) = self.start_seconds {
            data.insert(
                "start_seconds".into(),
                Value::Number(serde_json::Number::from(s)),
            );
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
        let id = event
            .data
            .get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        let total_seconds = event
            .data
            .get("total_seconds")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("total_seconds".into()))?;
        let name = event
            .data
            .get("name")
            .and_then(|v| v.as_str().map(String::from));
        let start_hours = event
            .data
            .get("start_hours")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let start_minutes = event
            .data
            .get("start_minutes")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let start_seconds = event
            .data
            .get("start_seconds")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        Ok(Self {
            id,
            total_seconds,
            name,
            start_hours,
            start_minutes,
            start_seconds,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerUpdated {
    pub id: String,
    pub is_active: bool,
    pub total_seconds: u64,
}

impl Eventable for TimerUpdated {
    const EVENT_TYPE: &'static str = "timer-updated";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("id".into(), Value::String(self.id));
        data.insert("is_active".into(), Value::Bool(self.is_active));
        data.insert(
            "total_seconds".into(),
            Value::Number(serde_json::Number::from(self.total_seconds)),
        );
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        let id = event
            .data
            .get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        let is_active = event
            .data
            .get("is_active")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| ConversionError::MissingField("is_active".into()))?;
        let total_seconds = event
            .data
            .get("total_seconds")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ConversionError::MissingField("total_seconds".into()))?;
        Ok(Self {
            id,
            is_active,
            total_seconds,
        })
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
        let id = event
            .data
            .get("id")
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
        let id = event
            .data
            .get("id")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| ConversionError::MissingField("id".into()))?;
        Ok(Self { id })
    }
}
