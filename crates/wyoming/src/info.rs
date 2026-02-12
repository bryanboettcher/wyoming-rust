use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Server asks the satellite to describe its capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Describe;

impl Eventable for Describe {
    const EVENT_TYPE: &'static str = "describe";

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

/// Satellite's response to a Describe request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    #[serde(default)]
    pub satellite: Option<SatelliteInfo>,
}

/// Information about this satellite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SatelliteInfo {
    pub name: String,
    #[serde(default)]
    pub area: Option<String>,
    #[serde(default)]
    pub has_mic: bool,
    #[serde(default)]
    pub has_snd: bool,
}

impl Eventable for Info {
    const EVENT_TYPE: &'static str = "info";

    fn into_event(self) -> Event {
        let data: Map<String, Value> = match serde_json::to_value(&self) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        Event::new(Self::EVENT_TYPE).with_data(data)
    }

    fn from_event(event: Event) -> Result<Self, ConversionError> {
        if event.event_type != Self::EVENT_TYPE {
            return Err(ConversionError::WrongType {
                expected: Self::EVENT_TYPE.into(),
                actual: event.event_type,
            });
        }
        serde_json::from_value(Value::Object(event.data))
            .map_err(|e| ConversionError::InvalidValue(e.to_string()))
    }
}
