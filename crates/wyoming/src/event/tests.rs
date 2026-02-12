use super::*;
use crate::audio::*;
use crate::asr::*;
use crate::info::*;
use crate::pipeline::*;
use crate::satellite::*;
use crate::timer::*;
use crate::tts::*;
use crate::wake::*;
use std::io::Cursor;

// ============================================================================
// Wire Format Round-Trip Tests (read_event / write_event)
// ============================================================================

#[test]
fn test_wire_format_no_data_no_payload() {
    // Event with no data and no payload (e.g., "describe")
    let event = Event::new("describe");

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded = read_event(&mut reader).unwrap();

    assert_eq!(decoded.event_type, "describe");
    assert!(decoded.data.is_empty());
    assert_eq!(decoded.payload, None);
}

#[test]
fn test_wire_format_with_data_no_payload() {
    // Event with data but no payload (e.g., "audio-start" with rate/width/channels)
    let mut data = Map::new();
    data.insert("rate".into(), Value::Number(16000.into()));
    data.insert("width".into(), Value::Number(2.into()));
    data.insert("channels".into(), Value::Number(1.into()));

    let event = Event::new("audio-start").with_data(data.clone());

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded = read_event(&mut reader).unwrap();

    assert_eq!(decoded.event_type, "audio-start");
    assert_eq!(decoded.data.get("rate").and_then(|v| v.as_u64()), Some(16000));
    assert_eq!(decoded.data.get("width").and_then(|v| v.as_u64()), Some(2));
    assert_eq!(decoded.data.get("channels").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(decoded.payload, None);
}

#[test]
fn test_wire_format_with_data_and_payload() {
    // Event with data AND payload (e.g., "audio-chunk" with format metadata + PCM bytes)
    let mut data = Map::new();
    data.insert("rate".into(), Value::Number(16000.into()));
    data.insert("width".into(), Value::Number(2.into()));
    data.insert("channels".into(), Value::Number(1.into()));

    let pcm_data = vec![0u8, 1, 2, 3, 4, 5, 6, 7];
    let event = Event::new("audio-chunk")
        .with_data(data.clone())
        .with_payload(pcm_data.clone());

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded = read_event(&mut reader).unwrap();

    assert_eq!(decoded.event_type, "audio-chunk");
    assert_eq!(decoded.data.get("rate").and_then(|v| v.as_u64()), Some(16000));
    assert_eq!(decoded.payload, Some(pcm_data));
}

#[test]
fn test_wire_format_payload_no_data() {
    // Event with payload but no data (edge case)
    let payload = vec![10u8, 20, 30, 40];
    let event = Event::new("custom-event").with_payload(payload.clone());

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded = read_event(&mut reader).unwrap();

    assert_eq!(decoded.event_type, "custom-event");
    assert!(decoded.data.is_empty());
    assert_eq!(decoded.payload, Some(payload));
}

#[test]
fn test_wire_format_multiple_events_sequence() {
    // Multiple events in sequence (write two events, read two events back)
    let event1 = Event::new("event-one");
    let event2 = Event::new("event-two").with_payload(vec![100, 200]);

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event1).unwrap();
    write_event(&mut buffer, &event2).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded1 = read_event(&mut reader).unwrap();
    let decoded2 = read_event(&mut reader).unwrap();

    assert_eq!(decoded1.event_type, "event-one");
    assert_eq!(decoded2.event_type, "event-two");
    assert_eq!(decoded2.payload, Some(vec![100, 200]));
}

#[test]
fn test_wire_format_connection_closed() {
    // Connection closed (empty read returns ConnectionClosed error)
    let buffer = Vec::new();
    let mut reader = Cursor::new(&buffer);

    let result = read_event(&mut reader);
    assert!(matches!(result, Err(ProtocolError::ConnectionClosed)));
}

#[test]
fn test_wire_format_malformed_json() {
    // Malformed JSON header returns error
    let buffer = b"not valid json\n";
    let mut reader = Cursor::new(&buffer[..]);

    let result = read_event(&mut reader);
    assert!(result.is_err());
    assert!(matches!(result, Err(ProtocolError::Json(_))));
}

#[test]
fn test_wire_format_missing_type_field() {
    // Header JSON missing required 'type' field
    let buffer = b"{\"version\":\"1.5.2\"}\n";
    let mut reader = Cursor::new(&buffer[..]);

    let result = read_event(&mut reader);
    assert!(matches!(result, Err(ProtocolError::InvalidHeader(_))));
}

// ============================================================================
// Wire Format Compatibility Tests
// ============================================================================

#[test]
fn test_wire_format_describe_exact_bytes() {
    // A "describe" event should produce a predictable byte sequence
    let event = Event::new("describe");

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let output = String::from_utf8(buffer).unwrap();
    // The header should contain these fields in JSON format
    assert!(output.contains("\"type\":\"describe\""));
    assert!(output.contains("\"version\":\"1.5.2\""));
    assert!(output.ends_with("\n"));

    // No data or payload sections should follow
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 1);
}

#[test]
fn test_wire_format_audio_chunk_predictable_structure() {
    // An "audio-chunk" with specific data should have predictable structure
    let mut data = Map::new();
    data.insert("rate".into(), Value::Number(8000.into()));
    data.insert("width".into(), Value::Number(1.into()));
    data.insert("channels".into(), Value::Number(1.into()));

    let payload = vec![0xAA, 0xBB, 0xCC, 0xDD];
    let event = Event::new("audio-chunk")
        .with_data(data)
        .with_payload(payload.clone());

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    // Verify structure: header line + data bytes + payload bytes
    let header_end = buffer.iter().position(|&b| b == b'\n').unwrap();
    let header = String::from_utf8(buffer[..header_end].to_vec()).unwrap();

    // Parse header to verify lengths
    let header_json: serde_json::Value = serde_json::from_str(&header).unwrap();
    let data_length = header_json["data_length"].as_u64().unwrap() as usize;
    let payload_length = header_json["payload_length"].as_u64().unwrap() as usize;

    assert_eq!(payload_length, 4);

    // Verify total length: header + \n + data + payload
    assert_eq!(buffer.len(), header_end + 1 + data_length + payload_length);

    // Verify payload is at the end
    let payload_start = buffer.len() - payload_length;
    assert_eq!(&buffer[payload_start..], &payload[..]);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Audio Events
// ============================================================================

#[test]
fn test_audio_start_round_trip() {
    let original = AudioStart {
        format: AudioFormat {
            rate: 16000,
            width: 2,
            channels: 1,
        },
        timestamp: Some(12345),
    };

    let event = original.clone().into_event();
    let decoded = AudioStart::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_audio_start_no_timestamp() {
    let original = AudioStart {
        format: AudioFormat::WYOMING_DEFAULT,
        timestamp: None,
    };

    let event = original.clone().into_event();
    let decoded = AudioStart::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_audio_chunk_round_trip() {
    let original = AudioChunk {
        format: AudioFormat {
            rate: 8000,
            width: 2,
            channels: 2,
        },
        audio: vec![1, 2, 3, 4, 5, 6, 7, 8],
        timestamp: Some(99999),
    };

    let event = original.clone().into_event();
    let decoded = AudioChunk::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_audio_chunk_from_pcm() {
    let pcm = vec![0xAA, 0xBB, 0xCC];
    let chunk = AudioChunk::from_pcm(pcm.clone());

    assert_eq!(chunk.audio, pcm);
    assert_eq!(chunk.format, AudioFormat::WYOMING_DEFAULT);
    assert_eq!(chunk.timestamp, None);
}

#[test]
fn test_audio_chunk_empty_payload() {
    let original = AudioChunk {
        format: AudioFormat::WYOMING_DEFAULT,
        audio: vec![],
        timestamp: None,
    };

    let event = original.clone().into_event();
    let decoded = AudioChunk::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_audio_stop_round_trip() {
    let original = AudioStop {
        timestamp: Some(54321),
    };

    let event = original.clone().into_event();
    let decoded = AudioStop::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_audio_stop_no_timestamp() {
    let original = AudioStop { timestamp: None };

    let event = original.clone().into_event();
    let decoded = AudioStop::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Wake Word Events
// ============================================================================

#[test]
fn test_detect_round_trip() {
    let original = Detect {
        names: Some(vec!["ok_nabu".into(), "hey_mycroft".into()]),
        context: Some(serde_json::json!({"key": "value"})),
    };

    let event = original.clone().into_event();
    let decoded = Detect::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_detect_minimal() {
    let original = Detect {
        names: None,
        context: None,
    };

    let event = original.clone().into_event();
    let decoded = Detect::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_detection_round_trip() {
    let original = Detection {
        name: Some("ok_nabu".into()),
        timestamp: Some(11111),
        speaker: Some("speaker-1".into()),
        context: Some(serde_json::json!({"detected": true})),
    };

    let event = original.clone().into_event();
    let decoded = Detection::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_detection_minimal() {
    let original = Detection {
        name: None,
        timestamp: None,
        speaker: None,
        context: None,
    };

    let event = original.clone().into_event();
    let decoded = Detection::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_not_detected_round_trip() {
    let original = NotDetected {
        context: Some(serde_json::json!({"timeout": 5000})),
    };

    let event = original.clone().into_event();
    let decoded = NotDetected::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_not_detected_no_context() {
    let original = NotDetected { context: None };

    let event = original.clone().into_event();
    let decoded = NotDetected::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - ASR Events
// ============================================================================

#[test]
fn test_transcribe_round_trip() {
    let original = Transcribe {
        name: Some("whisper".into()),
        language: Some("en".into()),
        context: Some(serde_json::json!({"model": "base"})),
    };

    let event = original.clone().into_event();
    let decoded = Transcribe::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_transcribe_minimal() {
    let original = Transcribe {
        name: None,
        language: None,
        context: None,
    };

    let event = original.clone().into_event();
    let decoded = Transcribe::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_transcript_round_trip() {
    let original = Transcript {
        text: "Hello, how are you?".into(),
        language: Some("en-US".into()),
        context: Some(serde_json::json!({"confidence": 0.95})),
    };

    let event = original.clone().into_event();
    let decoded = Transcript::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_transcript_minimal() {
    let original = Transcript {
        text: "Test".into(),
        language: None,
        context: None,
    };

    let event = original.clone().into_event();
    let decoded = Transcript::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - TTS Events
// ============================================================================

#[test]
fn test_synthesize_round_trip() {
    let original = Synthesize {
        text: "Hello world".into(),
        voice: Some(SynthesizeVoice {
            name: Some("Alan".into()),
            language: Some("en".into()),
            speaker: Some("speaker-1".into()),
        }),
    };

    let event = original.clone().into_event();
    let decoded = Synthesize::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_synthesize_minimal() {
    let original = Synthesize {
        text: "Test".into(),
        voice: None,
    };

    let event = original.clone().into_event();
    let decoded = Synthesize::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_synthesize_partial_voice() {
    let original = Synthesize {
        text: "Partial voice".into(),
        voice: Some(SynthesizeVoice {
            name: Some("Alan".into()),
            language: None,
            speaker: None,
        }),
    };

    let event = original.clone().into_event();
    let decoded = Synthesize::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Satellite Events
// ============================================================================

#[test]
fn test_run_satellite_round_trip() {
    let original = RunSatellite;
    let event = original.into_event();
    let decoded = RunSatellite::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_pause_satellite_round_trip() {
    let original = PauseSatellite;
    let event = original.into_event();
    let decoded = PauseSatellite::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_streaming_started_round_trip() {
    let original = StreamingStarted;
    let event = original.into_event();
    let decoded = StreamingStarted::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_streaming_stopped_round_trip() {
    let original = StreamingStopped;
    let event = original.into_event();
    let decoded = StreamingStopped::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_satellite_connected_round_trip() {
    let original = SatelliteConnected;
    let event = original.into_event();
    let decoded = SatelliteConnected::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_satellite_disconnected_round_trip() {
    let original = SatelliteDisconnected;
    let event = original.into_event();
    let decoded = SatelliteDisconnected::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_played_round_trip() {
    let original = Played;
    let event = original.into_event();
    let decoded = Played::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Info Events
// ============================================================================

#[test]
fn test_describe_round_trip() {
    let original = Describe;
    let event = original.into_event();
    let decoded = Describe::from_event(event).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_info_round_trip() {
    let original = Info {
        satellite: Some(SatelliteInfo {
            name: "test-satellite".into(),
            area: Some("living-room".into()),
            has_mic: true,
            has_snd: true,
        }),
    };

    let event = original.clone().into_event();
    let decoded = Info::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_info_no_satellite() {
    let original = Info { satellite: None };

    let event = original.clone().into_event();
    let decoded = Info::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_info_minimal_satellite() {
    let original = Info {
        satellite: Some(SatelliteInfo {
            name: "minimal".into(),
            area: None,
            has_mic: false,
            has_snd: false,
        }),
    };

    let event = original.clone().into_event();
    let decoded = Info::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Pipeline Events
// ============================================================================

#[test]
fn test_run_pipeline_round_trip() {
    let original = RunPipeline {
        start_stage: PipelineStage::Wake,
        end_stage: PipelineStage::Tts,
        wake_word_name: Some("ok_nabu".into()),
        restart_on_end: true,
    };

    let event = original.clone().into_event();
    let decoded = RunPipeline::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_run_pipeline_minimal() {
    let original = RunPipeline {
        start_stage: PipelineStage::Asr,
        end_stage: PipelineStage::Handle,
        wake_word_name: None,
        restart_on_end: false,
    };

    let event = original.clone().into_event();
    let decoded = RunPipeline::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_pipeline_stage_conversions() {
    assert_eq!(PipelineStage::Wake.as_str(), "wake");
    assert_eq!(PipelineStage::Asr.as_str(), "asr");
    assert_eq!(PipelineStage::Intent.as_str(), "intent");
    assert_eq!(PipelineStage::Handle.as_str(), "handle");
    assert_eq!(PipelineStage::Tts.as_str(), "tts");

    assert_eq!(PipelineStage::from_str("wake"), Some(PipelineStage::Wake));
    assert_eq!(PipelineStage::from_str("asr"), Some(PipelineStage::Asr));
    assert_eq!(PipelineStage::from_str("intent"), Some(PipelineStage::Intent));
    assert_eq!(PipelineStage::from_str("handle"), Some(PipelineStage::Handle));
    assert_eq!(PipelineStage::from_str("tts"), Some(PipelineStage::Tts));
    assert_eq!(PipelineStage::from_str("invalid"), None);
}

// ============================================================================
// Eventable Trait Round-Trip Tests - Timer Events
// ============================================================================

#[test]
fn test_timer_started_round_trip() {
    let original = TimerStarted {
        id: "timer-123".into(),
        total_seconds: 60.5,
        name: Some("Kitchen Timer".into()),
    };

    let event = original.clone().into_event();
    let decoded = TimerStarted::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_timer_started_no_name() {
    let original = TimerStarted {
        id: "timer-456".into(),
        total_seconds: 120.0,
        name: None,
    };

    let event = original.clone().into_event();
    let decoded = TimerStarted::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_timer_updated_round_trip() {
    let original = TimerUpdated {
        id: "timer-789".into(),
        is_active: true,
        total_seconds: 45.25,
    };

    let event = original.clone().into_event();
    let decoded = TimerUpdated::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_timer_updated_inactive() {
    let original = TimerUpdated {
        id: "timer-inactive".into(),
        is_active: false,
        total_seconds: 0.0,
    };

    let event = original.clone().into_event();
    let decoded = TimerUpdated::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_timer_cancelled_round_trip() {
    let original = TimerCancelled {
        id: "timer-cancel".into(),
    };

    let event = original.clone().into_event();
    let decoded = TimerCancelled::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_timer_finished_round_trip() {
    let original = TimerFinished {
        id: "timer-done".into(),
    };

    let event = original.clone().into_event();
    let decoded = TimerFinished::from_event(event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Error Case Tests
// ============================================================================

#[test]
fn test_from_event_wrong_type() {
    let event = Event::new("audio-start");
    let result = AudioChunk::from_event(event);

    assert!(result.is_err());
    match result {
        Err(ConversionError::WrongType { expected, actual }) => {
            assert_eq!(expected, "audio-chunk");
            assert_eq!(actual, "audio-start");
        }
        _ => panic!("Expected WrongType error"),
    }
}

#[test]
fn test_from_event_missing_required_field() {
    // AudioStart requires rate, width, channels
    let event = Event::new("audio-start");
    let result = AudioStart::from_event(event);

    assert!(result.is_err());
    match result {
        Err(ConversionError::MissingField(field)) => {
            assert_eq!(field, "rate");
        }
        _ => panic!("Expected MissingField error"),
    }
}

#[test]
fn test_from_event_missing_text_field() {
    // Transcript requires text field
    let event = Event::new("transcript");
    let result = Transcript::from_event(event);

    assert!(result.is_err());
    match result {
        Err(ConversionError::MissingField(field)) => {
            assert_eq!(field, "text");
        }
        _ => panic!("Expected MissingField error"),
    }
}

#[test]
fn test_from_event_missing_pipeline_fields() {
    // RunPipeline requires start_stage and end_stage
    let event = Event::new("run-pipeline");
    let result = RunPipeline::from_event(event);

    assert!(result.is_err());
    match result {
        Err(ConversionError::MissingField(field)) => {
            assert_eq!(field, "start_stage");
        }
        _ => panic!("Expected MissingField error"),
    }
}

#[test]
fn test_wrong_type_satellite_events() {
    let event = Event::new("pause-satellite");
    let result = RunSatellite::from_event(event);

    assert!(result.is_err());
    assert!(matches!(result, Err(ConversionError::WrongType { .. })));
}

#[test]
fn test_wrong_type_timer_events() {
    let event = Event::new("timer-finished");
    let result = TimerStarted::from_event(event);

    assert!(result.is_err());
    assert!(matches!(result, Err(ConversionError::WrongType { .. })));
}

// ============================================================================
// Event Type Constant Tests
// ============================================================================

#[test]
fn test_event_type_constants() {
    assert_eq!(AudioStart::EVENT_TYPE, "audio-start");
    assert_eq!(AudioChunk::EVENT_TYPE, "audio-chunk");
    assert_eq!(AudioStop::EVENT_TYPE, "audio-stop");
    assert_eq!(Detect::EVENT_TYPE, "detect");
    assert_eq!(Detection::EVENT_TYPE, "detection");
    assert_eq!(NotDetected::EVENT_TYPE, "not-detected");
    assert_eq!(Transcribe::EVENT_TYPE, "transcribe");
    assert_eq!(Transcript::EVENT_TYPE, "transcript");
    assert_eq!(Synthesize::EVENT_TYPE, "synthesize");
    assert_eq!(RunSatellite::EVENT_TYPE, "run-satellite");
    assert_eq!(PauseSatellite::EVENT_TYPE, "pause-satellite");
    assert_eq!(StreamingStarted::EVENT_TYPE, "streaming-started");
    assert_eq!(StreamingStopped::EVENT_TYPE, "streaming-stopped");
    assert_eq!(SatelliteConnected::EVENT_TYPE, "satellite-connected");
    assert_eq!(SatelliteDisconnected::EVENT_TYPE, "satellite-disconnected");
    assert_eq!(Played::EVENT_TYPE, "played");
    assert_eq!(Describe::EVENT_TYPE, "describe");
    assert_eq!(Info::EVENT_TYPE, "info");
    assert_eq!(RunPipeline::EVENT_TYPE, "run-pipeline");
    assert_eq!(TimerStarted::EVENT_TYPE, "timer-started");
    assert_eq!(TimerUpdated::EVENT_TYPE, "timer-updated");
    assert_eq!(TimerCancelled::EVENT_TYPE, "timer-cancelled");
    assert_eq!(TimerFinished::EVENT_TYPE, "timer-finished");
}

// ============================================================================
// Integration: Wire Format + Eventable Round-Trip
// ============================================================================

#[test]
fn test_full_round_trip_audio_chunk_wire_and_eventable() {
    // Create a typed AudioChunk
    let original_chunk = AudioChunk {
        format: AudioFormat {
            rate: 22050,
            width: 2,
            channels: 2,
        },
        audio: vec![0xFF, 0xEE, 0xDD, 0xCC],
        timestamp: Some(98765),
    };

    // Convert to Event
    let event = original_chunk.clone().into_event();

    // Write to wire format
    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    // Read from wire format
    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();

    // Convert back to typed AudioChunk
    let decoded_chunk = AudioChunk::from_event(decoded_event).unwrap();

    // Verify full round-trip
    assert_eq!(decoded_chunk, original_chunk);
}

#[test]
fn test_full_round_trip_info_wire_and_eventable() {
    let original_info = Info {
        satellite: Some(SatelliteInfo {
            name: "wyoming-satellite-1".into(),
            area: Some("bedroom".into()),
            has_mic: true,
            has_snd: false,
        }),
    };

    let event = original_info.clone().into_event();

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();

    let decoded_info = Info::from_event(decoded_event).unwrap();

    assert_eq!(decoded_info, original_info);
}

#[test]
fn test_full_round_trip_run_pipeline() {
    let original = RunPipeline {
        start_stage: PipelineStage::Wake,
        end_stage: PipelineStage::Handle,
        wake_word_name: Some("jarvis".into()),
        restart_on_end: true,
    };

    let event = original.clone().into_event();

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();

    let decoded = RunPipeline::from_event(decoded_event).unwrap();

    assert_eq!(decoded, original);
}

// ============================================================================
// Python Wire Format Compatibility Tests (Golden Bytes)
// ============================================================================
// These construct raw wire bytes matching what the Python Wyoming server
// would actually send, then verify our Rust parser handles them correctly.

#[test]
fn test_python_compat_describe() {
    let wire = b"{\"type\": \"describe\", \"version\": \"1.5.2\"}\n";
    let mut reader = Cursor::new(&wire[..]);
    let event = read_event(&mut reader).unwrap();

    assert_eq!(event.event_type, "describe");
    assert!(event.data.is_empty());
    assert_eq!(event.payload, None);
}

#[test]
fn test_python_compat_audio_start() {
    let data_json = b"{\"rate\": 16000, \"width\": 2, \"channels\": 1}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"audio-start\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let audio_start = AudioStart::from_event(event).unwrap();

    assert_eq!(audio_start.format.rate, 16000);
    assert_eq!(audio_start.format.width, 2);
    assert_eq!(audio_start.format.channels, 1);
    assert_eq!(audio_start.timestamp, None);
}

#[test]
fn test_python_compat_audio_chunk_with_payload() {
    let data_json = b"{\"rate\": 16000, \"width\": 2, \"channels\": 1}";
    let payload = vec![0x01, 0x02, 0x03, 0x04];
    let data_len = data_json.len();
    let payload_len = payload.len();
    let header = format!(
        "{{\"type\": \"audio-chunk\", \"version\": \"1.5.2\", \"data_length\": {}, \"payload_length\": {}}}\n",
        data_len, payload_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);
    wire.extend_from_slice(&payload);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let chunk = AudioChunk::from_event(event).unwrap();

    assert_eq!(chunk.format.rate, 16000);
    assert_eq!(chunk.audio, payload);
}

#[test]
fn test_python_compat_detection() {
    let data_json = b"{\"name\": \"ok_nabu\", \"timestamp\": 12345}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"detection\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let detection = Detection::from_event(event).unwrap();

    assert_eq!(detection.name, Some("ok_nabu".into()));
    assert_eq!(detection.timestamp, Some(12345));
}

#[test]
fn test_python_compat_synthesize_with_voice() {
    let data_json = b"{\"text\": \"hello\", \"voice\": {\"name\": \"piper\"}}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"synthesize\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let synth = Synthesize::from_event(event).unwrap();

    assert_eq!(synth.text, "hello");
    let voice = synth.voice.unwrap();
    assert_eq!(voice.name, Some("piper".into()));
    assert_eq!(voice.language, None);
    assert_eq!(voice.speaker, None);
}

#[test]
fn test_python_compat_run_pipeline() {
    let data_json = b"{\"start_stage\": \"asr\", \"end_stage\": \"tts\", \"restart_on_end\": true}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"run-pipeline\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let pipeline = RunPipeline::from_event(event).unwrap();

    assert_eq!(pipeline.start_stage, PipelineStage::Asr);
    assert_eq!(pipeline.end_stage, PipelineStage::Tts);
    assert!(pipeline.restart_on_end);
}

#[test]
fn test_python_compat_info_with_satellite() {
    let data_json = b"{\"satellite\": {\"name\": \"my-sat\", \"area\": \"kitchen\", \"has_mic\": true, \"has_snd\": false}}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"info\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let info = Info::from_event(event).unwrap();

    let sat = info.satellite.unwrap();
    assert_eq!(sat.name, "my-sat");
    assert_eq!(sat.area, Some("kitchen".into()));
    assert!(sat.has_mic);
    assert!(!sat.has_snd);
}

// ============================================================================
// into_event Output Structure Tests (Non-Round-Trip)
// ============================================================================

#[test]
fn test_audio_start_into_event_structure() {
    let audio_start = AudioStart {
        format: AudioFormat {
            rate: 16000,
            width: 2,
            channels: 1,
        },
        timestamp: Some(999),
    };

    let event = audio_start.into_event();

    assert_eq!(event.event_type, "audio-start");
    assert_eq!(event.data.get("rate").and_then(|v| v.as_u64()), Some(16000));
    assert_eq!(event.data.get("width").and_then(|v| v.as_u64()), Some(2));
    assert_eq!(event.data.get("channels").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(event.data.get("timestamp").and_then(|v| v.as_u64()), Some(999));
    assert!(event.data.get("rate").unwrap().is_number());
    assert_eq!(event.payload, None);
}

#[test]
fn test_audio_chunk_into_event_structure() {
    let chunk = AudioChunk {
        format: AudioFormat {
            rate: 22050,
            width: 2,
            channels: 2,
        },
        audio: vec![1, 2, 3, 4],
        timestamp: None,
    };

    let event = chunk.into_event();

    assert_eq!(event.event_type, "audio-chunk");
    assert_eq!(event.payload, Some(vec![1, 2, 3, 4]));
    assert_eq!(event.data.get("rate").and_then(|v| v.as_u64()), Some(22050));
    assert!(event.data.get("timestamp").is_none());
}

#[test]
fn test_synthesize_into_event_voice_nested_object() {
    let synth = Synthesize {
        text: "hello".into(),
        voice: Some(SynthesizeVoice {
            name: Some("piper".into()),
            language: Some("en-US".into()),
            speaker: Some("p270".into()),
        }),
    };

    let event = synth.into_event();

    assert_eq!(event.event_type, "synthesize");
    assert_eq!(event.data.get("text").and_then(|v| v.as_str()), Some("hello"));
    let voice = event.data.get("voice").expect("voice key must exist");
    let voice_obj = voice.as_object().expect("voice must be an object");
    assert_eq!(voice_obj.get("name").and_then(|v| v.as_str()), Some("piper"));
    assert_eq!(voice_obj.get("language").and_then(|v| v.as_str()), Some("en-US"));
    assert_eq!(voice_obj.get("speaker").and_then(|v| v.as_str()), Some("p270"));
    // Flat dot-notation keys must NOT exist
    assert!(event.data.get("voice.name").is_none());
    assert!(event.data.get("voice.language").is_none());
    assert!(event.data.get("voice.speaker").is_none());
}

#[test]
fn test_transcript_into_event_structure() {
    let transcript = Transcript {
        text: "turn on the lights".into(),
        language: Some("en".into()),
        context: None,
    };

    let event = transcript.into_event();

    assert_eq!(event.event_type, "transcript");
    assert_eq!(event.data.get("text").and_then(|v| v.as_str()), Some("turn on the lights"));
    assert_eq!(event.data.get("language").and_then(|v| v.as_str()), Some("en"));
    assert!(event.data.get("text").unwrap().is_string());
    assert!(event.data.get("context").is_none());
    assert_eq!(event.payload, None);
}

#[test]
fn test_run_pipeline_into_event_structure() {
    let pipeline = RunPipeline {
        start_stage: PipelineStage::Wake,
        end_stage: PipelineStage::Tts,
        wake_word_name: Some("hey_jarvis".into()),
        restart_on_end: true,
    };

    let event = pipeline.into_event();

    assert_eq!(event.event_type, "run-pipeline");
    assert_eq!(event.data.get("start_stage").and_then(|v| v.as_str()), Some("wake"));
    assert_eq!(event.data.get("end_stage").and_then(|v| v.as_str()), Some("tts"));
    assert_eq!(event.data.get("wake_word_name").and_then(|v| v.as_str()), Some("hey_jarvis"));
    assert_eq!(event.data.get("restart_on_end").and_then(|v| v.as_bool()), Some(true));
}

// ============================================================================
// Wire Format Header Field Tests
// ============================================================================

#[test]
fn test_write_event_no_data_omits_data_length() {
    let event = Event::new("describe");

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let header_end = buffer.iter().position(|&b| b == b'\n').unwrap();
    let header_str = String::from_utf8(buffer[..header_end].to_vec()).unwrap();
    let header: serde_json::Value = serde_json::from_str(&header_str).unwrap();

    assert!(header.get("data_length").is_none());
    assert!(header.get("payload_length").is_none());
    assert_eq!(header["type"].as_str(), Some("describe"));
    assert_eq!(header["version"].as_str(), Some("1.5.2"));
}

#[test]
fn test_write_event_data_length_matches_actual_data_bytes() {
    let mut data = Map::new();
    data.insert("key".into(), Value::String("value".into()));
    let event = Event::new("test").with_data(data);

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let header_end = buffer.iter().position(|&b| b == b'\n').unwrap();
    let header_str = String::from_utf8(buffer[..header_end].to_vec()).unwrap();
    let header: serde_json::Value = serde_json::from_str(&header_str).unwrap();

    let declared_data_length = header["data_length"].as_u64().unwrap() as usize;
    let actual_data_bytes = &buffer[header_end + 1..];

    assert_eq!(actual_data_bytes.len(), declared_data_length);
    let parsed: serde_json::Value = serde_json::from_slice(actual_data_bytes).unwrap();
    assert_eq!(parsed["key"].as_str(), Some("value"));
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_unicode_in_transcript_text() {
    let original = Transcript {
        text: "\u{1F44B} Hej v\u{00E4}rlden! \u{4F60}\u{597D}".into(),
        language: Some("multi".into()),
        context: None,
    };

    let event = original.clone().into_event();
    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();
    let decoded = Transcript::from_event(decoded_event).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_empty_string_through_wire() {
    let original = Transcript {
        text: "".into(),
        language: None,
        context: None,
    };

    let event = original.clone().into_event();
    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();
    let decoded = Transcript::from_event(decoded_event).unwrap();

    assert_eq!(decoded.text, "");
}

#[test]
fn test_text_with_newlines() {
    let original = Transcript {
        text: "line one\nline two\nline three".into(),
        language: None,
        context: None,
    };

    let event = original.clone().into_event();
    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded_event = read_event(&mut reader).unwrap();
    let decoded = Transcript::from_event(decoded_event).unwrap();

    assert_eq!(decoded, original);
    assert!(decoded.text.contains('\n'));
}

#[test]
fn test_large_payload() {
    let large_payload: Vec<u8> = (0..131072).map(|i| (i % 256) as u8).collect();
    let event = Event::new("audio-chunk").with_payload(large_payload.clone());

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let mut reader = Cursor::new(&buffer);
    let decoded = read_event(&mut reader).unwrap();

    assert_eq!(decoded.payload, Some(large_payload));
}

#[test]
fn test_forward_compat_extra_header_fields() {
    let wire = b"{\"type\": \"describe\", \"version\": \"2.0.0\", \"new_field\": true, \"another\": 42}\n";
    let mut reader = Cursor::new(&wire[..]);
    let event = read_event(&mut reader).unwrap();

    assert_eq!(event.event_type, "describe");
}

#[test]
fn test_forward_compat_extra_data_fields() {
    let data_json = b"{\"rate\": 16000, \"width\": 2, \"channels\": 1, \"future_field\": \"ignored\"}";
    let data_len = data_json.len();
    let header = format!(
        "{{\"type\": \"audio-start\", \"version\": \"1.5.2\", \"data_length\": {}}}\n",
        data_len
    );
    let mut wire: Vec<u8> = Vec::new();
    wire.extend_from_slice(header.as_bytes());
    wire.extend_from_slice(data_json);

    let mut reader = Cursor::new(&wire);
    let event = read_event(&mut reader).unwrap();
    let audio_start = AudioStart::from_event(event).unwrap();

    assert_eq!(audio_start.format.rate, 16000);
}

#[test]
fn test_no_version_field() {
    let wire = b"{\"type\": \"describe\"}\n";
    let mut reader = Cursor::new(&wire[..]);
    let event = read_event(&mut reader).unwrap();

    assert_eq!(event.event_type, "describe");
}

#[test]
fn test_truncated_data_section_returns_error() {
    let wire = b"{\"type\": \"test\", \"data_length\": 100}\nabcde";
    let mut reader = Cursor::new(&wire[..]);
    let result = read_event(&mut reader);

    assert!(result.is_err());
}

#[test]
fn test_truncated_payload_returns_error() {
    let wire = b"{\"type\": \"test\", \"payload_length\": 100}\nabc";
    let mut reader = Cursor::new(&wire[..]);
    let result = read_event(&mut reader);

    assert!(result.is_err());
}

#[test]
fn test_type_field_as_number_fails() {
    let wire = b"{\"type\": 42, \"version\": \"1.5.2\"}\n";
    let mut reader = Cursor::new(&wire[..]);
    let result = read_event(&mut reader);

    assert!(matches!(result, Err(ProtocolError::InvalidHeader(_))));
}

#[test]
fn test_multiple_events_with_payloads_dont_bleed() {
    let event1 = Event::new("evt-1").with_payload(vec![0xAA; 100]);
    let event2 = Event::new("evt-2").with_payload(vec![0xBB; 50]);
    let event3 = Event::new("evt-3");

    let mut buffer = Vec::new();
    write_event(&mut buffer, &event1).unwrap();
    write_event(&mut buffer, &event2).unwrap();
    write_event(&mut buffer, &event3).unwrap();

    let mut reader = Cursor::new(&buffer);
    let d1 = read_event(&mut reader).unwrap();
    let d2 = read_event(&mut reader).unwrap();
    let d3 = read_event(&mut reader).unwrap();

    assert_eq!(d1.event_type, "evt-1");
    assert_eq!(d1.payload, Some(vec![0xAA; 100]));
    assert_eq!(d2.event_type, "evt-2");
    assert_eq!(d2.payload, Some(vec![0xBB; 50]));
    assert_eq!(d3.event_type, "evt-3");
    assert_eq!(d3.payload, None);

    assert!(matches!(read_event(&mut reader), Err(ProtocolError::ConnectionClosed)));
}

#[test]
fn test_rust_output_parseable_by_python() {
    let audio_start = AudioStart {
        format: AudioFormat {
            rate: 16000,
            width: 2,
            channels: 1,
        },
        timestamp: None,
    };

    let event = audio_start.into_event();
    let mut buffer = Vec::new();
    write_event(&mut buffer, &event).unwrap();

    let newline_pos = buffer.iter().position(|&b| b == b'\n').unwrap();
    let header_str = std::str::from_utf8(&buffer[..newline_pos]).unwrap();
    let header: serde_json::Value = serde_json::from_str(header_str).unwrap();

    assert!(header["type"].is_string());
    assert_eq!(header["type"].as_str().unwrap(), "audio-start");

    let data_length = header["data_length"].as_u64().unwrap() as usize;
    let data_bytes = &buffer[newline_pos + 1..newline_pos + 1 + data_length];
    let data: serde_json::Value = serde_json::from_slice(data_bytes).unwrap();

    assert!(data["rate"].is_number());
    assert_eq!(data["rate"].as_u64().unwrap(), 16000);
    assert_eq!(buffer.len(), newline_pos + 1 + data_length);
}
