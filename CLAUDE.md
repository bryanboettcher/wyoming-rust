# Wyoming-Rust

Rust implementation of a Wyoming protocol satellite for Home Assistant voice pipelines.
Targets Raspberry Pi Zero W v1.1 (ARM1176, 512MB RAM) as minimum viable platform.

## Architecture Reference

- ADR: /home/insta/src/bryanboettcher/homelab/documentation/adr/adr-011-rust-wyoming-satellite.md
- Design doc: docs/architecture.md (protocol types, state machine, main loop, traits)

## Key Design Decisions

- **No async/tokio.** Single-threaded blocking I/O. Mic read (~20ms) is the clock tick.
- **Protocol crate (`wyoming`)** is a reusable library. Satellite binary is separate.
- **State machine is a pure function**: `(state, input) → (new_state, actions[])`. No I/O in transitions.
- **Traits for hardware abstraction**: `AudioSource`, `AudioSink`, `Led`, `Gpio` — swap real hardware for test doubles.
- **Wire format**: JSON header line + \n + data bytes + payload bytes. NOT binary framing.

## Wyoming Protocol Wire Format

```
Header: {"type":"audio-chunk","version":"1.5.2","data_length":N,"payload_length":M}\n
Data:   N bytes of UTF-8 JSON (domain-specific fields)
Payload: M bytes of raw binary (PCM audio, etc.)
```

## Cross-Compilation Target

`arm-unknown-linux-gnueabihf` (ARMv6, hard float) via `cross` or cargo with appropriate linker.

## Conventions

- Prefer `std::net` blocking I/O over async
- Prefer `serde_json` for protocol serialization
- Minimize allocations on the audio hot path (reuse buffers where possible)
- All hardware interaction behind traits for testability
