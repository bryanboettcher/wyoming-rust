use crate::event::{ConversionError, Event, Eventable};

/// Server informs the satellite that the voice pipeline is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSatellite;

impl Eventable for RunSatellite {
    const EVENT_TYPE: &'static str = "run-satellite";

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

/// Server informs the satellite that the voice pipeline is not ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseSatellite;

impl Eventable for PauseSatellite {
    const EVENT_TYPE: &'static str = "pause-satellite";

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

/// Satellite has started streaming audio to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingStarted;

impl Eventable for StreamingStarted {
    const EVENT_TYPE: &'static str = "streaming-started";

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

/// Satellite has stopped streaming audio to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingStopped;

impl Eventable for StreamingStopped {
    const EVENT_TYPE: &'static str = "streaming-stopped";

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

/// Satellite has connected to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatelliteConnected;

impl Eventable for SatelliteConnected {
    const EVENT_TYPE: &'static str = "satellite-connected";

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

/// Satellite has disconnected from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatelliteDisconnected;

impl Eventable for SatelliteDisconnected {
    const EVENT_TYPE: &'static str = "satellite-disconnected";

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

/// Playback of audio has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Played;

impl Eventable for Played {
    const EVENT_TYPE: &'static str = "played";

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
