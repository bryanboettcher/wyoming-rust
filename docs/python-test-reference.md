# Python Wyoming Test Reference

This document contains test data extracted from the upstream Python Wyoming library that can be used to validate wire-level compatibility of the Rust implementation.

## Wire Format Specification (from test_event.py)

### Complete Message Structure

Every Wyoming protocol message consists of three parts transmitted sequentially:

```
1. JSON header line (UTF-8) + \n
2. Data section (UTF-8 JSON bytes, length = data_length from header)
3. Payload section (raw binary bytes, length = payload_length from header)
```

### Example: Complete Event with Data and Payload

**Test constants from Python:**
```python
PAYLOAD = b"test\npayload"
DATA = {"test": "data"}
DATA_BYTES = json.dumps(DATA, ensure_ascii=False).encode("utf-8")  # {"test":"data"}
```

**Wire format bytes:**
```
{"type":"test-event","version":"1.5.2","data_length":16,"payload_length":12}\n
{"test":"data"}
test\npayload
```

**Verification assertions:**
```python
# Header line is JSON with type, version, and lengths
assert json.loads(header_line) == {
    "type": "test-event",
    "version": wyoming_version,  # "1.5.2" as of Feb 2026
    "data_length": 16,  # len(b'{"test":"data"}')
    "payload_length": 12,  # len(b'test\npayload')
}

# Data section is raw UTF-8 JSON bytes (NOT a string)
assert data_bytes == b'{"test":"data"}'

# Payload is raw binary
assert payload_bytes == b"test\npayload"
```

### Example: Event with Data Only (No Payload)

**Wire format:**
```
{"type":"test-event","version":"1.5.2","data_length":16}\n
{"test":"data"}
```

**Verification:**
- `payload_length` field is omitted from header (not set to 0)
- No payload bytes follow the data section

### Example: Minimal Event (No Data, No Payload)

**Wire format:**
```
{"type":"test-event","version":"1.5.2"}\n
```

**Verification:**
- Both `data_length` and `payload_length` omitted
- No data or payload bytes follow

### Inline Data Merging

The header JSON can contain an optional `"data"` field for short metadata. This is merged with the data section, with the data section taking precedence:

**Header with inline data:**
```json
{
    "type": "test-event",
    "version": "1.5.2",
    "data_length": 16,
    "payload_length": 12,
    "data": {
        "test": "this will be overwritten by DATA",
        "test2": "this will not"
    }
}
```

**Data section:**
```json
{"test": "data"}
```

**Final merged Event.data:**
```python
{
    "test": "data",           # Overwritten by data section
    "test2": "this will not"  # Preserved from header
}
```

## Audio Events (from shared.py and test_audio.py)

### AudioStart Event

```python
AUDIO_START = AudioStart(rate=16000, width=2, channels=1)
```

**Wire format:**
```
{"type":"audio-start","version":"1.5.2","data_length":N}\n
{"rate":16000,"width":2,"channels":1}
```

**Field definitions:**
- `rate`: Sample rate in Hz (int)
- `width`: Bytes per sample (int) - 2 for 16-bit PCM
- `channels`: Number of audio channels (int) - 1 for mono, 2 for stereo
- `timestamp`: Optional int (milliseconds)

### AudioChunk Event

```python
AUDIO_CHUNK = AudioChunk(
    rate=16000,
    width=2,
    channels=1,
    audio=bytes([255] * 960)  # 30ms of audio at 16kHz mono 16-bit
)
```

**Calculation:** 30ms × 16000 Hz ÷ 1000 ms/s × 2 bytes/sample = 960 bytes

**Wire format:**
```
{"type":"audio-chunk","version":"1.5.2","data_length":N,"payload_length":960}\n
{"rate":16000,"width":2,"channels":1}
<960 bytes of PCM audio>
```

**Field definitions:**
- `rate`, `width`, `channels`: Same as AudioStart
- `timestamp`: Optional int (milliseconds)
- Payload: Raw PCM audio bytes

### AudioStop Event

```python
AUDIO_STOP = AudioStop()
```

**Wire format:**
```
{"type":"audio-stop","version":"1.5.2","data_length":N}\n
{"timestamp":null}
```

Or if timestamp omitted:
```
{"type":"audio-stop","version":"1.5.2","data_length":2}\n
{}
```

**Field definitions:**
- `timestamp`: Optional int (milliseconds)

## ASR Events (from wyoming/asr.py)

### Transcript Event (ASR → Satellite)

```python
Transcript(text="test", language=None, context=None)
```

**Wire format:**
```
{"type":"transcript","version":"1.5.2","data_length":N}\n
{"text":"test"}
```

**Full example with all fields:**
```python
Transcript(
    text="turn on the lights",
    language="en-US",
    context={"conversation_id": "abc123"}
)
```

**Wire format:**
```
{"type":"transcript","version":"1.5.2","data_length":N}\n
{"text":"turn on the lights","language":"en-US","context":{"conversation_id":"abc123"}}
```

**Field definitions:**
- `text`: Transcribed text (str, required)
- `language`: Language code (Optional[str])
- `context`: Arbitrary context dict (Optional[Dict])

### Transcribe Event (Satellite → ASR)

```python
Transcribe(name=None, language=None, context=None)
```

**Wire format:**
```
{"type":"transcribe","version":"1.5.2","data_length":2}\n
{}
```

**Field definitions:**
- `name`: ASR model name (Optional[str])
- `language`: Expected language (Optional[str])
- `context`: Context from previous interaction (Optional[Dict])

## Wake Word Events (from wyoming/wake.py)

### Detection Event (Wake → Satellite)

```python
Detection(name=None, timestamp=None, speaker=None, context=None)
```

**Minimal wire format:**
```
{"type":"detection","version":"1.5.2","data_length":2}\n
{}
```

**Full example:**
```python
Detection(
    name="ok_nabu",
    timestamp=1234567890,
    speaker="user1",
    context={"confidence": 0.95}
)
```

**Wire format:**
```
{"type":"detection","version":"1.5.2","data_length":N}\n
{"name":"ok_nabu","timestamp":1234567890,"speaker":"user1","context":{"confidence":0.95}}
```

**Field definitions:**
- `name`: Detected wake word model name (Optional[str])
- `timestamp`: Audio chunk timestamp (Optional[int])
- `speaker`: Speaker identifier (Optional[str])
- `context`: Arbitrary metadata (Optional[Dict])

### Detect Event (Satellite → Wake)

```python
Detect(names=None, context=None)
```

**Wire format:**
```
{"type":"detect","version":"1.5.2","data_length":2}\n
{}
```

**With wake word filter:**
```python
Detect(names=["ok_nabu", "hey_jarvis"])
```

**Wire format:**
```
{"type":"detect","version":"1.5.2","data_length":N}\n
{"names":["ok_nabu","hey_jarvis"]}
```

**Field definitions:**
- `names`: List of wake word models to detect (Optional[List[str]]) - None = any
- `context`: Arbitrary metadata (Optional[Dict])

### NotDetected Event

```python
NotDetected(context=None)
```

**Wire format:**
```
{"type":"not-detected","version":"1.5.2","data_length":2}\n
{}
```

## TTS Events (from wyoming/tts.py)

### Synthesize Event (Server → Satellite)

```python
Synthesize(text="test", voice=None, context=None)
```

**Minimal wire format:**
```
{"type":"synthesize","version":"1.5.2","data_length":N}\n
{"text":"test"}
```

**Full example:**
```python
Synthesize(
    text="Hello, world!",
    voice=SynthesizeVoice(name="en_US-lessac-medium", language="en-US", speaker=None),
    context={"request_id": "xyz"}
)
```

**Wire format:**
```
{"type":"synthesize","version":"1.5.2","data_length":N}\n
{"text":"Hello, world!","voice":{"name":"en_US-lessac-medium","language":"en-US"},"context":{"request_id":"xyz"}}
```

**Field definitions:**
- `text`: Text to synthesize (str, required)
- `voice`: Voice configuration (Optional[SynthesizeVoice])
  - `name`: Voice model name (Optional[str])
  - `language`: Language code (Optional[str])
  - `speaker`: Speaker variant (Optional[str])
- `context`: Arbitrary metadata (Optional[Dict])

## Satellite Control Events (from wyoming/satellite.py)

### RunSatellite Event (Server → Satellite)

```python
RunSatellite()
```

**Wire format:**
```
{"type":"run-satellite","version":"1.5.2","data_length":2}\n
{}
```

**Purpose:** Informs satellite that server is ready to run pipelines (satellite should start listening for wake words)

### PauseSatellite Event (Server → Satellite)

```python
PauseSatellite()
```

**Wire format:**
```
{"type":"pause-satellite","version":"1.5.2","data_length":2}\n
{}
```

**Purpose:** Informs satellite that server is not ready (satellite should pause wake word detection)

### StreamingStarted Event (Satellite → Server)

```python
StreamingStarted()
```

**Wire format:**
```
{"type":"streaming-started","version":"1.5.2","data_length":2}\n
{}
```

**Purpose:** Satellite has started streaming audio to server for ASR

### StreamingStopped Event (Satellite → Server)

```python
StreamingStopped()
```

**Wire format:**
```
{"type":"streaming-stopped","version":"1.5.2","data_length":2}\n
{}
```

**Purpose:** Satellite has stopped streaming audio to server

### SatelliteConnected Event

```python
SatelliteConnected()
```

**Wire format:**
```
{"type":"satellite-connected","version":"1.5.2","data_length":2}\n
{}
```

### SatelliteDisconnected Event

```python
SatelliteDisconnected()
```

**Wire format:**
```
{"type":"satellite-disconnected","version":"1.5.2","data_length":2}\n
{}
```

## Pipeline Events (from wyoming/pipeline.py)

### RunPipeline Event (Satellite → Server)

From test_satellite.py, when wake word is detected:

```python
RunPipeline(start_stage=PipelineStage.ASR, end_stage=PipelineStage.TTS)
```

**Wire format:**
```
{"type":"run-pipeline","version":"1.5.2","data_length":N}\n
{"start_stage":"asr","end_stage":"tts"}
```

**Pipeline stages:**
- `"asr"`: Automatic Speech Recognition
- `"intent"`: Intent recognition
- `"handle"`: Intent handling
- `"tts"`: Text-to-Speech

**Common patterns:**
- Full voice assistant: `start_stage="asr"`, `end_stage="tts"`
- Transcription only: `start_stage="asr"`, `end_stage="asr"`
- TTS only: `start_stage="tts"`, `end_stage="tts"`

## Integration Test Flow (from test_satellite.py)

Complete message exchange for a wake-word-triggered voice interaction:

```
1. Server → Satellite: RunSatellite()
   Satellite starts listening for wake words

2. Satellite → Server: Detection()
   Wake word detected

3. Satellite → Server: RunPipeline(start_stage="asr", end_stage="tts")
   Request pipeline execution

4. Satellite → Server: StreamingStarted()
   (implicitly sent before audio)

5. Satellite → Server: AudioChunk() × N
   30ms audio chunks sent continuously

6. Server → Satellite: Transcript(text="test")
   ASR result

7. Satellite → Server: AudioChunk() × N (continues until VAD stops)
   More audio chunks

8. Satellite → Server: StreamingStopped()
   Audio streaming complete

9. Server → Satellite: Synthesize(text="test")
   TTS request (response to transcript)

10. Server → Satellite: AudioStart()
    TTS audio beginning

11. Server → Satellite: AudioChunk() × N
    TTS audio data

12. Server → Satellite: AudioStop()
    TTS audio complete

Satellite plays audio and returns to listening state
```

## Event Client Observations (from test_satellite.py)

The satellite can forward events to an optional event service:

**Events forwarded to event service:**
- Detection
- StreamingStarted
- StreamingStopped
- Transcript
- Synthesize
- AudioStart (TTS)
- AudioStop (TTS)

**NOT forwarded:**
- AudioChunk (neither mic nor TTS)
- RunPipeline

## Critical Implementation Notes

### UTF-8 Encoding
All JSON data (both header and data section) must use UTF-8 encoding:
```python
json.dumps(data, ensure_ascii=False).encode("utf-8")
```

### Newline Handling
- Header line MUST be terminated with `\n` (0x0A byte)
- Newlines within payload are preserved as-is (e.g., `b"test\npayload"`)

### Zero-Length Sections
- If `data_length` is 0 or omitted: data section is `{}` (empty JSON object)
- If `payload_length` is omitted: no payload bytes
- Fields with value `None` are typically omitted from JSON (not sent as `null`)

### Version String
As of February 2026, the Python library version is `"1.5.2"`. This appears in every event header.

### Type Checking Pattern
Python code uses `.is_type()` class methods:
```python
if AudioChunk.is_type(event.type):
    chunk = AudioChunk.from_event(event)
```

Rust equivalent should match against event type strings exactly:
```rust
match event.event_type.as_str() {
    "audio-chunk" => { /* ... */ }
    "audio-start" => { /* ... */ }
    // etc.
}
```

## Test Data for Validation

### Minimal Valid Events (type only)
```
{"type":"describe","version":"1.5.2"}\n
{"type":"audio-stop","version":"1.5.2","data_length":2}\n{}
{"type":"run-satellite","version":"1.5.2","data_length":2}\n{}
```

### Audio Chunk (30ms @ 16kHz mono 16-bit)
```python
rate = 16000
width = 2
channels = 1
duration_ms = 30
audio_bytes = bytes([255] * 960)  # 960 = 30 * 16000 / 1000 * 2 * 1
```

Wire format:
```
{"type":"audio-chunk","version":"1.5.2","data_length":45,"payload_length":960}\n
{"rate":16000,"width":2,"channels":1,"timestamp":null}
<960 bytes of 0xFF>
```

### Edge Cases to Test

1. **Empty JSON object in data section:**
   ```
   {"type":"test","version":"1.5.2","data_length":2}\n
   {}
   ```

2. **Inline data merge:**
   Header: `{"type":"test","version":"1.5.2","data_length":16,"data":{"a":1,"b":2}}\n`
   Data: `{"b":99,"c":3}`
   Result: `{"a":1,"b":99,"c":3}`

3. **Payload with embedded newlines:**
   ```
   {"type":"test","version":"1.5.2","data_length":2,"payload_length":12}\n
   {}
   test\npayload
   ```

4. **Unicode in data section:**
   ```python
   DATA = {"text": "Hello 世界"}
   DATA_BYTES = json.dumps(DATA, ensure_ascii=False).encode("utf-8")
   # Results in multi-byte UTF-8 sequence
   ```

## Recommended Validation Tests for Rust Implementation

1. **Round-trip serialization:** Serialize an event, parse it back, verify equality
2. **Wire format byte matching:** Generate events and compare exact bytes with Python output
3. **Parse Python test vectors:** Use the exact test data from test_event.py
4. **Integration test:** Connect to Python Wyoming server, exchange messages
5. **Edge cases:** Empty data, no payload, inline data merge, unicode, binary payload with newlines

## Source Files Reference

- Wire format tests: `rhasspy/wyoming/tests/test_event.py`
- Audio tests: `rhasspy/wyoming/tests/test_audio.py`
- Integration tests: `rhasspy/wyoming-satellite/tests/test_satellite.py`
- Test constants: `rhasspy/wyoming-satellite/tests/shared.py`
- Event implementation: `rhasspy/wyoming/wyoming/event.py`
- Type definitions:
  - `rhasspy/wyoming/wyoming/audio.py`
  - `rhasspy/wyoming/wyoming/asr.py`
  - `rhasspy/wyoming/wyoming/wake.py`
  - `rhasspy/wyoming/wyoming/tts.py`
  - `rhasspy/wyoming/wyoming/satellite.py`
  - `rhasspy/wyoming/wyoming/info.py`
