# Wyoming-Rust Architecture

## Wire Format

The Wyoming protocol uses a three-part TCP framing. No binary length-prefix — just a JSON line followed by optional sections.

```
Part 1 - Header line (UTF-8 JSON + \n):
  {"type":"audio-chunk","version":"1.5.2","data_length":52,"payload_length":640}\n

Part 2 - Data section (data_length bytes of UTF-8 JSON, no trailing newline):
  {"rate":16000,"width":2,"channels":1,"timestamp":12345}

Part 3 - Payload section (payload_length bytes of raw binary):
  <640 bytes of PCM audio>
```

- `data_length` and `payload_length` default to 0 if absent
- Header always ends with `\n`
- Data section has no delimiter — length-counted
- Payload section has no delimiter — length-counted
- Events with no data and no payload are just the header line

## Event Type Inventory

### Audio (bidirectional)
| Type | Data Fields | Payload | Direction |
|------|------------|---------|-----------|
| `audio-start` | rate, width, channels, timestamp? | none | both |
| `audio-chunk` | rate, width, channels, timestamp? | PCM bytes | both |
| `audio-stop` | timestamp? | none | both |

### Wake Word (server → satellite)
| Type | Data Fields | Payload | Direction |
|------|------------|---------|-----------|
| `detect` | names[]?, context? | none | sat → server (request) |
| `detection` | name?, timestamp?, speaker? | none | server → sat |
| `not-detected` | context? | none | server → sat |

### ASR (server → satellite)
| Type | Data Fields | Payload |
|------|------------|---------|
| `transcribe` | name?, language?, context? | none |
| `transcript` | text, language?, context? | none |

### TTS (server → satellite)
| Type | Data Fields | Payload |
|------|------------|---------|
| `synthesize` | text, voice? | none |

### Satellite Lifecycle
| Type | Data Fields | Direction |
|------|------------|-----------|
| `run-satellite` | (empty) | server → sat |
| `pause-satellite` | (empty) | server → sat |
| `streaming-started` | (empty) | sat → server |
| `streaming-stopped` | (empty) | sat → server |

### Service Discovery
| Type | Data Fields | Direction |
|------|------------|-----------|
| `describe` | (empty) | server → sat |
| `info` | asr[], tts[], wake[], satellite? | sat → server |

### Playback
| Type | Data Fields | Direction |
|------|------------|-----------|
| `played` | (empty) | sat → server |

### Pipeline
| Type | Data Fields | Direction |
|------|------------|-----------|
| `run-pipeline` | start_stage, end_stage, wake_word_name? | server → sat |

### Timer (stretch goal)
| Type | Data Fields | Direction |
|------|------------|-----------|
| `timer-started` | id, total_seconds, name? | server → sat |
| `timer-updated` | id, is_active, total_seconds | server → sat |
| `timer-cancelled` | id | server → sat |
| `timer-finished` | id | server → sat |

### Handle (server-side, included for protocol completeness)
| Type | Data Fields |
|------|------------|
| `handled` | text?, context? |
| `not-handled` | text?, context? |

## Crate Layout

```
wyoming-rust/
├── Cargo.toml                        # Workspace root
├── crates/
│   ├── wyoming/                      # Protocol library (reusable)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                # Re-exports
│   │       ├── event.rs              # RawEvent + TCP read/write
│   │       ├── audio.rs              # AudioStart, AudioChunk, AudioStop
│   │       ├── wake.rs               # Detect, Detection, NotDetected
│   │       ├── asr.rs                # Transcribe, Transcript
│   │       ├── tts.rs                # Synthesize
│   │       ├── satellite.rs          # RunSatellite, PauseSatellite, etc.
│   │       ├── info.rs               # Describe, Info
│   │       ├── pipeline.rs           # RunPipeline
│   │       └── timer.rs              # Timer events (stretch)
│   └── satellite/                    # Binary
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs               # Entry point, main loop
│           ├── config.rs             # TOML config
│           ├── state.rs              # State machine + transitions
│           ├── connection.rs         # TCP to HA, handshake, reconnect
│           ├── audio_source.rs       # trait AudioSource: ALSA mic or WAV file
│           ├── audio_sink.rs         # trait AudioSink: ALSA speaker or file
│           ├── led.rs                # WS2812 LED control (trait + impl)
│           └── gpio.rs               # Schmitt trigger input (trait + impl)
├── tests/                            # Integration tests
│   ├── fixtures/                     # Captured wire-format bytes
│   └── protocol_compat.rs           # Round-trip tests vs Python
└── docker/
    └── docker-compose.yml            # Test Wyoming services
```

## Protocol Crate: Key Types

### RawEvent (the wire type)

```rust
/// An event as it appears on the wire. Untyped.
/// Analogous to a raw HTTP request before routing.
pub struct Event {
    pub event_type: String,
    pub data: serde_json::Map<String, Value>,
    pub payload: Option<Vec<u8>>,
}

/// Read one event from a TCP stream.
/// Blocks until a complete event is available.
pub fn read_event(reader: &mut impl BufRead) -> Result<Event, ProtocolError>;

/// Write one event to a TCP stream.
pub fn write_event(writer: &mut impl Write, event: &Event) -> Result<(), ProtocolError>;
```

### Eventable trait (typed conversion)

```rust
/// Converts between a typed domain struct and a raw Event.
/// Every Wyoming message type implements this.
///
/// C# equivalent: interface IEventable<T> {
///     static string EventType { get; }
///     Event ToEvent();
///     static T FromEvent(Event e);
/// }
pub trait Eventable: Sized {
    const EVENT_TYPE: &'static str;
    fn into_event(self) -> Event;
    fn from_event(event: Event) -> Result<Self, ConversionError>;
}
```

### Audio types

```rust
/// Shared format metadata for all audio events.
pub struct AudioFormat {
    pub rate: u32,       // Sample rate in Hz (16000)
    pub width: u16,      // Bytes per sample (2 = 16-bit)
    pub channels: u16,   // Channel count (1 = mono)
}

pub struct AudioStart {
    pub format: AudioFormat,
    pub timestamp: Option<u64>,
}

pub struct AudioChunk {
    pub format: AudioFormat,
    pub audio: Vec<u8>,          // Raw PCM payload
    pub timestamp: Option<u64>,
}

pub struct AudioStop {
    pub timestamp: Option<u64>,
}
```

### Wake word types

```rust
pub struct Detect {
    pub names: Option<Vec<String>>,  // Which wake words to listen for (None = any)
    pub context: Option<Value>,
}

pub struct Detection {
    pub name: Option<String>,       // Which wake word was detected
    pub timestamp: Option<u64>,
    pub speaker: Option<String>,    // Speaker identification (optional)
    pub context: Option<Value>,
}
```

### Info / Describe

```rust
/// Server asks "what are you?"
pub struct Describe;  // No fields

/// Satellite responds with its capabilities.
pub struct Info {
    pub satellite: Option<SatelliteInfo>,
    // We declare ourselves as a satellite with optional mic/snd
}

pub struct SatelliteInfo {
    pub name: String,
    pub area: Option<String>,        // Room name
    pub has_mic: bool,
    pub has_snd: bool,
}
```

## Satellite Crate: State Machine

### States

```rust
/// Each variant is a distinct operating mode with different I/O behavior.
///
/// C# equivalent: sealed abstract record SatelliteState
///     with derived records Idle, Streaming, etc.
pub enum SatelliteState {
    /// No sound detected. Blocking on GPIO interrupt + server socket.
    /// No audio capture, no streaming, no server load.
    Idle,

    /// Schmitt trigger fired. Capturing mic audio and streaming to HA.
    /// Server running wake word detection on incoming audio.
    Streaming,

    /// Server detected wake word. LED → cyan. Still streaming audio
    /// so server can run STT on the utterance that follows.
    Triggered,

    /// Server processing: STT → intent → TTS. LED → blue pulse.
    /// Still streaming (server may need more audio).
    Processing,

    /// Playing TTS audio through speaker. LED → green.
    /// Mic muted (half-duplex). Receiving audio-chunk from server.
    Responding,
}
```

### Transition inputs

```rust
/// Events that drive state transitions.
/// Sourced from hardware, network, or internal timers.
pub enum SatelliteInput {
    // Hardware
    GpioHigh,               // Schmitt trigger fired
    GpioLow,                // Schmitt trigger released
    SilenceTimeout,         // No GPIO re-trigger for N seconds
    PlaybackComplete,       // Speaker finished playing TTS

    // Server messages (mapped from Wyoming events)
    ServerRunSatellite,
    ServerPauseSatellite,
    ServerDetection(Detection),
    ServerVoiceStarted,     // Pipeline started processing
    ServerTtsStart(AudioStart),
    ServerTtsChunk(AudioChunk),
    ServerTtsStop,
    ServerVoiceStopped,     // Pipeline finished

    // Connection
    Connected,
    Disconnected,
}
```

### Transition outputs (actions)

```rust
/// Side effects the main loop must execute after a state transition.
pub enum Action {
    StartCapture,            // Open mic, begin reading frames
    StopCapture,             // Close mic
    SendAudioStart,          // Send audio-start event to server
    SendAudioStop,           // Send audio-stop event to server
    SendStreamingStarted,
    SendStreamingStopped,
    StartPlayback,           // Open speaker
    StopPlayback,            // Close speaker
    PlayAudioChunk(Vec<u8>), // Write PCM to speaker
    SetLed(LedState),        // Update LED color/pattern
    Reconnect,               // Re-establish TCP connection
}

pub enum LedState {
    Off,
    DimWhite,
    Cyan,
    BluePulse,
    Green,
    RedBlink,
}
```

### Transition function

```rust
/// Pure function: (current_state, input) → (new_state, actions[])
/// No I/O, no side effects. Fully testable.
pub fn transition(
    state: &SatelliteState,
    input: &SatelliteInput,
) -> (SatelliteState, Vec<Action>) {
    use SatelliteState::*;
    use SatelliteInput::*;

    match (state, input) {
        // ── IDLE ──────────────────────────────────────────
        (Idle, GpioHigh) => (
            Streaming,
            vec![Action::StartCapture, Action::SendAudioStart, Action::SendStreamingStarted],
        ),

        // ── STREAMING ─────────────────────────────────────
        (Streaming, SilenceTimeout) => (
            Idle,
            vec![Action::StopCapture, Action::SendAudioStop, Action::SendStreamingStopped,
                 Action::SetLed(LedState::Off)],
        ),
        (Streaming, ServerDetection(det)) => (
            Triggered,
            vec![Action::SetLed(LedState::Cyan)],
        ),
        (Streaming, ServerPauseSatellite) => (
            Idle,
            vec![Action::StopCapture, Action::SendAudioStop, Action::SendStreamingStopped],
        ),

        // ── TRIGGERED ─────────────────────────────────────
        (Triggered, ServerVoiceStarted) => (
            Processing,
            vec![Action::SetLed(LedState::BluePulse)],
        ),

        // ── PROCESSING ────────────────────────────────────
        (Processing, ServerTtsStart(_)) => (
            Responding,
            vec![Action::StopCapture, Action::SetLed(LedState::Green), Action::StartPlayback],
        ),

        // ── RESPONDING ────────────────────────────────────
        (Responding, ServerTtsChunk(chunk)) => (
            Responding,
            vec![Action::PlayAudioChunk(chunk.audio.clone())],
        ),
        (Responding, ServerTtsStop) | (Responding, PlaybackComplete) => (
            Idle,
            vec![Action::StopPlayback, Action::SendStreamingStopped,
                 Action::SetLed(LedState::Off)],
        ),

        // ── ANY STATE: connection loss ────────────────────
        (_, Disconnected) => (
            Idle,
            vec![Action::StopCapture, Action::StopPlayback,
                 Action::SetLed(LedState::RedBlink), Action::Reconnect],
        ),

        // Default: no transition
        (current, _) => (current.clone(), vec![]),
    }
}
```

## Main Loop Design

Single-threaded. The blocking operation changes based on current state.

```rust
fn run(config: &Config) -> Result<()> {
    let mut conn = Connection::connect(&config.server)?;
    let mut mic: Box<dyn AudioSource> = open_audio_source(config);
    let mut spk: Box<dyn AudioSink> = open_audio_sink(config);
    let mut gpio = Gpio::open(&config.gpio)?;
    let mut led = Led::open(&config.led)?;

    // Handshake: respond to 'describe' with 'info'
    conn.handle_describe(&config.satellite_info)?;
    // Block until server sends 'run-satellite'
    conn.wait_for_run()?;

    let mut state = SatelliteState::Idle;

    loop {
        // Gather next input based on current state
        let input = match &state {
            // IDLE: block on GPIO edge or server message (poll/select both fds)
            Idle => poll_idle(&conn, &gpio, config.keepalive_timeout)?,

            // STREAMING/TRIGGERED/PROCESSING: mic read is the ~20ms clock tick
            Streaming | Triggered | Processing => {
                // 1. Read one mic frame (blocks ~20ms)
                let frame = mic.read_frame()?;
                conn.send(AudioChunk::from_pcm(&config.audio_format, frame))?;

                // 2. Check GPIO
                if !gpio.is_high() && last_gpio.elapsed() > config.silence_timeout {
                    SatelliteInput::SilenceTimeout
                }
                // 3. Check for server message (non-blocking)
                else if let Some(event) = conn.try_read()? {
                    map_server_event(event)
                } else {
                    continue; // No transition, loop back to read next frame
                }
            }

            // RESPONDING: block on server socket (receiving TTS chunks)
            Responding => {
                let event = conn.read_event()?;
                map_server_event(event)
            }
        };

        // Pure state transition
        let (new_state, actions) = transition(&state, &input);

        // Execute side effects
        for action in actions {
            execute_action(action, &mut conn, &mut mic, &mut spk, &mut led)?;
        }

        state = new_state;
    }
}
```

### Why single-threaded works

| State | Blocks on | Duration | Why it's fine |
|-------|-----------|----------|---------------|
| Idle | GPIO edge + TCP (via poll) | Indefinitely | Nothing to do until sound or server message |
| Streaming | Mic read | ~20ms | Natural clock tick; check GPIO + server between frames |
| Triggered | Mic read | ~20ms | Same as streaming |
| Processing | Mic read | ~20ms | Same — server may still need audio for STT |
| Responding | TCP read | Varies | Receiving TTS chunks; mic is muted anyway |

No threads, no async, no tokio. The single core alternates between I/O sources based on state.

## Audio Source / Sink Traits

```rust
/// Abstraction over mic input. Allows swapping ALSA for WAV file in testing.
///
/// C# equivalent: interface IAudioSource {
///     byte[] ReadFrame();
///     void Start();
///     void Stop();
/// }
pub trait AudioSource {
    fn read_frame(&mut self) -> Result<Vec<u8>, AudioError>;
    fn start(&mut self) -> Result<(), AudioError>;
    fn stop(&mut self) -> Result<(), AudioError>;
}

/// Abstraction over speaker output.
pub trait AudioSink {
    fn write_frame(&mut self, pcm: &[u8]) -> Result<(), AudioError>;
    fn start(&mut self) -> Result<(), AudioError>;
    fn stop(&mut self) -> Result<(), AudioError>;
}
```

Implementations:
- `AlsaSource` / `AlsaSink` — real hardware (WM8960 via I2S)
- `WavFileSource` — reads a .wav file as if it were a mic (for testing)
- `WavFileSink` — writes received TTS to a .wav file (for testing)
- `NullSink` — discards audio (for protocol-only testing)

## Config (satellite.toml)

```toml
[satellite]
name = "living-room"
area = "Living Room"

[server]
host = "homeassistant.local"
port = 10700

[audio]
device = "hw:0,0"          # ALSA device
rate = 16000
width = 2
channels = 1
chunk_ms = 20              # ~320 samples per frame

[gpio]
vad_pin = 17               # Schmitt trigger input
silence_timeout_ms = 2500  # How long after GPIO low before stopping stream

[led]
pin = 10                   # SPI MOSI for WS2812
method = "spi"             # "spi" or "pwm"

# For testing without hardware:
# [audio]
# wav_input = "test.wav"
# wav_output = "tts_output.wav"
```

## Testing Strategy

### Unit tests (protocol crate)
- Round-trip: construct typed event → into_event() → write to buffer → read from buffer → from_event() → compare
- Wire-format fixtures: captured bytes from Python implementation → read_event() → verify fields
- Edge cases: events with no data, no payload, empty strings, large payloads

### Integration tests (satellite crate)
- State machine: feed SatelliteInput sequences, verify state transitions and action lists
- Pure functions, no I/O needed

### Docker integration
- docker-compose with wyoming-openwakeword + wyoming-piper
- Rust satellite connects, streams WAV file, receives detection + TTS
- Validates full protocol flow end-to-end

### Protocol capture
- Run Python wyoming-satellite against test server, tcpdump the TCP stream
- Parse captured bytes into fixture files for unit tests
- Guarantees wire-format compatibility
