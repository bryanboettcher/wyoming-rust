use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Voice configuration for TTS synthesis.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizeVoice {
    pub name: Option<String>,
    pub language: Option<String>,
    pub speaker: Option<String>,
}

/// Request to synthesize speech from text.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesize {
    pub text: String,
    pub voice: Option<SynthesizeVoice>,
    pub context: Option<Value>,
}

impl Eventable for Synthesize {
    const EVENT_TYPE: &'static str = "synthesize";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert("text".into(), Value::String(self.text));
        if let Some(voice) = self.voice {
            let mut voice_map = Map::new();
            if let Some(name) = voice.name {
                voice_map.insert("name".into(), Value::String(name));
            }
            if let Some(language) = voice.language {
                voice_map.insert("language".into(), Value::String(language));
            }
            if let Some(speaker) = voice.speaker {
                voice_map.insert("speaker".into(), Value::String(speaker));
            }
            data.insert("voice".into(), Value::Object(voice_map));
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

        let voice = event.data.get("voice").and_then(|v| {
            let voice_obj = v.as_object()?;
            let name = voice_obj.get("name").and_then(|v| v.as_str().map(String::from));
            let language = voice_obj.get("language").and_then(|v| v.as_str().map(String::from));
            let speaker = voice_obj.get("speaker").and_then(|v| v.as_str().map(String::from));
            if name.is_some() || language.is_some() || speaker.is_some() {
                Some(SynthesizeVoice { name, language, speaker })
            } else {
                None
            }
        });

        let context = event.data.get("context").cloned();

        Ok(Self { text, voice, context })
    }
}

// ============================================================================
// Streaming Synthesize types
// ============================================================================

/// Start streaming synthesis. Event type: "synthesize-start"
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizeStart {
    pub voice: Option<SynthesizeVoice>,
    pub context: Option<Value>,
}

impl Eventable for SynthesizeStart {
    const EVENT_TYPE: &'static str = "synthesize-start";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        if let Some(voice) = self.voice {
            let mut voice_map = Map::new();
            if let Some(name) = voice.name {
                voice_map.insert("name".into(), Value::String(name));
            }
            if let Some(language) = voice.language {
                voice_map.insert("language".into(), Value::String(language));
            }
            if let Some(speaker) = voice.speaker {
                voice_map.insert("speaker".into(), Value::String(speaker));
            }
            data.insert("voice".into(), Value::Object(voice_map));
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
        let voice = event.data.get("voice").and_then(|v| {
            let voice_obj = v.as_object()?;
            let name = voice_obj.get("name").and_then(|v| v.as_str().map(String::from));
            let language = voice_obj.get("language").and_then(|v| v.as_str().map(String::from));
            let speaker = voice_obj.get("speaker").and_then(|v| v.as_str().map(String::from));
            if name.is_some() || language.is_some() || speaker.is_some() {
                Some(SynthesizeVoice { name, language, speaker })
            } else {
                None
            }
        });
        let context = event.data.get("context").cloned();
        Ok(Self { voice, context })
    }
}

/// A chunk of text for streaming synthesis. Event type: "synthesize-chunk"
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizeChunk {
    pub text: String,
}

impl Eventable for SynthesizeChunk {
    const EVENT_TYPE: &'static str = "synthesize-chunk";

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

/// Stop streaming synthesis. Event type: "synthesize-stop"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynthesizeStop;

impl Eventable for SynthesizeStop {
    const EVENT_TYPE: &'static str = "synthesize-stop";

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

/// Server has stopped synthesis. Event type: "synthesize-stopped"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynthesizeStopped;

impl Eventable for SynthesizeStopped {
    const EVENT_TYPE: &'static str = "synthesize-stopped";

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
