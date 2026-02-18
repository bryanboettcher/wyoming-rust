use serde_json::{Map, Value};

use crate::event::{ConversionError, Event, Eventable};

/// Pipeline stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStage {
    Wake,
    Asr,
    Intent,
    Handle,
    Tts,
}

impl PipelineStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Wake => "wake",
            Self::Asr => "asr",
            Self::Intent => "intent",
            Self::Handle => "handle",
            Self::Tts => "tts",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "wake" => Some(Self::Wake),
            "asr" => Some(Self::Asr),
            "intent" => Some(Self::Intent),
            "handle" => Some(Self::Handle),
            "tts" => Some(Self::Tts),
            _ => None,
        }
    }
}

/// Request to run a voice pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPipeline {
    pub start_stage: PipelineStage,
    pub end_stage: PipelineStage,
    pub wake_word_name: Option<String>,
    pub restart_on_end: bool,
    pub wake_word_names: Option<Vec<String>>,
    pub announce_text: Option<String>,
}

impl Eventable for RunPipeline {
    const EVENT_TYPE: &'static str = "run-pipeline";

    fn into_event(self) -> Event {
        let mut data = Map::new();
        data.insert(
            "start_stage".into(),
            Value::String(self.start_stage.as_str().into()),
        );
        data.insert(
            "end_stage".into(),
            Value::String(self.end_stage.as_str().into()),
        );
        if let Some(name) = self.wake_word_name {
            data.insert("wake_word_name".into(), Value::String(name));
        }
        if self.restart_on_end {
            data.insert("restart_on_end".into(), Value::Bool(true));
        }
        if let Some(names) = self.wake_word_names {
            data.insert(
                "wake_word_names".into(),
                Value::Array(names.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(text) = self.announce_text {
            data.insert("announce_text".into(), Value::String(text));
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
        let start_stage = event
            .data
            .get("start_stage")
            .and_then(|v| v.as_str())
            .and_then(PipelineStage::from_str)
            .ok_or_else(|| ConversionError::MissingField("start_stage".into()))?;
        let end_stage = event
            .data
            .get("end_stage")
            .and_then(|v| v.as_str())
            .and_then(PipelineStage::from_str)
            .ok_or_else(|| ConversionError::MissingField("end_stage".into()))?;
        let wake_word_name = event
            .data
            .get("wake_word_name")
            .and_then(|v| v.as_str().map(String::from));
        let restart_on_end = event
            .data
            .get("restart_on_end")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let wake_word_names = event.data.get("wake_word_names").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        });
        let announce_text = event
            .data
            .get("announce_text")
            .and_then(|v| v.as_str().map(String::from));

        Ok(Self {
            start_stage,
            end_stage,
            wake_word_name,
            restart_on_end,
            wake_word_names,
            announce_text,
        })
    }
}
