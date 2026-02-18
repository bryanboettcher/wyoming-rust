use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::audio::AudioFormat;
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

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asr: Vec<AsrProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tts: Vec<TtsProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handle: Vec<HandleProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent: Vec<IntentProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wake: Vec<WakeProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mic: Vec<MicProgram>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snd: Vec<SndProgram>,
}

/// Attribution metadata for an artifact (required by HA's Wyoming Python library).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Attribution {
    pub name: String,
    pub url: String,
}

/// Information about this satellite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SatelliteInfo {
    pub name: String,
    #[serde(default)]
    pub attribution: Attribution,
    #[serde(default)]
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_vad: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_wake_words: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_wake_words: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_trigger: Option<bool>,
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

// ============================================================================
// Macro to generate artifact-based structs with standard base fields
// ============================================================================

/// Generate a struct with the 5 standard artifact fields (name, attribution,
/// installed, description, version) plus any additional domain-specific fields.
macro_rules! define_artifact {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$field_meta:meta])* pub $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            pub name: String,
            #[serde(default)]
            pub attribution: Attribution,
            #[serde(default)]
            pub installed: bool,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub version: Option<String>,
            $( $(#[$field_meta])* pub $field : $ty, )*
        }
    };
}

// ============================================================================
// ASR types
// ============================================================================

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AsrModel {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub languages: Vec<String>,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AsrProgram {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub models: Vec<AsrModel>,
        #[serde(default)]
        pub supports_transcript_streaming: bool,
    }
}

// ============================================================================
// TTS types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsVoiceSpeaker {
    pub name: String,
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct TtsVoice {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub languages: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub speakers: Option<Vec<TtsVoiceSpeaker>>,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct TtsProgram {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub voices: Vec<TtsVoice>,
        #[serde(default)]
        pub supports_synthesize_streaming: bool,
    }
}

// ============================================================================
// Handle types
// ============================================================================

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct HandleModel {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub languages: Vec<String>,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct HandleProgram {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub models: Vec<HandleModel>,
        #[serde(default)]
        pub supports_handled_streaming: bool,
    }
}

// ============================================================================
// Wake types
// ============================================================================

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WakeModel {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub languages: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub phrase: Option<String>,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WakeProgram {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub models: Vec<WakeModel>,
    }
}

// ============================================================================
// Intent types
// ============================================================================

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct IntentModel {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub languages: Vec<String>,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct IntentProgram {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub models: Vec<IntentModel>,
    }
}

// ============================================================================
// Mic / Snd types
// ============================================================================

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MicProgram {
        pub mic_format: AudioFormat,
    }
}

define_artifact! {
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct SndProgram {
        pub snd_format: AudioFormat,
    }
}
