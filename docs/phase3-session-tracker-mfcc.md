# Session Tracker + MFCC Pipeline Stage

## Context

The satellite has server-side minute rollups tracking energy, noise floor, and VAD trigger stats. Two gaps remain:

1. **Session-level visibility**: VAD triggers are tracked, but not the *sessions* they produce. A session is the full Idle→Streaming→[...]→Idle lifecycle. Overnight false triggers show up as high trigger counts in rollups, but there's no way to distinguish "HVAC made 40 useless sub-second sessions" from "someone had 5 real conversations." Capturing session records with onset stats and outcomes enables overnight forensics.

2. **Spectral features**: Energy, ZCR, and bandpass ratio are time-domain or narrowband metrics. MFCCs provide a compact spectral fingerprint of each frame — the foundation for a future wake word pre-filter, and immediately useful as spectral summary stats (centroid, flatness) that distinguish speech from broadband noise in rollups.

---

## Part 1: Session Tracker

### Data Structures (in `diagnostics.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionOutcome {
    SilenceTimeout,   // Streaming→Idle without reaching Processing
    Completed,        // Reached Processing or Responding
    Paused,           // ServerPauseSatellite during Streaming
    Disconnected,     // Connection lost
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRecord {
    pub onset_uptime_ms: u64,
    pub duration_ms: u64,
    pub onset_energy: f64,
    pub peak_energy: f64,
    pub onset_zcr: f64,
    pub mean_zcr: f64,
    pub onset_speech_band: f64,
    pub mean_speech_band: f64,
    pub noise_floor: f64,
    pub peak_state: String,       // highest state reached ("Streaming", "Processing", etc.)
    pub outcome: SessionOutcome,
}
```

Ring buffer: `VecDeque<SessionRecord>`, max 10,000 entries (~800KB). At typical rates (5 sessions/min), covers 33+ hours.

### Active Session Accumulator

```rust
struct ActiveSession {
    onset_uptime_ms: u64,
    onset_energy: f64,        // filled on first tick after onset
    onset_zcr: f64,
    onset_speech_band: f64,
    noise_floor: f64,
    peak_energy: f64,
    zcr_sum: f64,
    speech_band_sum: f64,
    frame_count: u32,
    peak_state: SatelliteState,
    needs_onset_stats: bool,  // true until first tick fills onset values
}
```

### SessionTracker

```rust
pub struct SessionTracker {
    sessions: VecDeque<SessionRecord>,
    max_sessions: usize,           // 10,000
    active: Option<ActiveSession>,
}
```

Methods:
- `start_session(uptime_ms)` — called on Idle→Streaming transition
- `end_session(uptime_ms, outcome)` — called on any→Idle transition while active
- `feed_tick(energy, zcr, speech_band, floor)` — called every tick during active session; fills onset stats on first call
- `update_peak_state(state)` — called on state transitions during active session

### Where It Plugs In

**`main.rs:run_session()`** (line 167, state change block):
```rust
if new_state != state {
    let mut d = diagnostics.lock().unwrap();
    d.last_state_change = Instant::now();
    d.on_state_change(&state, &new_state);  // NEW — drives session tracker
    // ... existing interaction_count logic ...
}
```

`DiagnosticsState::on_state_change()`:
- `Idle → Streaming` → `session_tracker.start_session(uptime_ms)`
- `Any → Idle` (while active) → `session_tracker.end_session(uptime_ms, outcome)` + `rollup_accumulator.record_session(dur, is_false)`
- Any transition during active session → `session_tracker.update_peak_state(new_state)`

Outcome determination:
- `SilenceTimeout`: `peak_state == Streaming` (never got past streaming)
- `Completed`: `peak_state ∈ {Processing, Responding}`
- `Paused`: triggered by `ServerPauseSatellite` (peak_state == Streaming, but old state was Streaming and action list would contain pause)
- `Disconnected`: triggered by `Disconnected` input

Actually, we can determine outcome from old_state + new_state in the transition. If ending in Idle:
- If previous peak_state was only Streaming or Triggered → `SilenceTimeout` (most common false trigger)
- If peak_state reached Processing or Responding → `Completed`

For Paused vs SilenceTimeout: both end with peak_state=Streaming. We can distinguish by passing the input that caused the transition. But to keep it simple, we'll just use `SilenceTimeout` for both (pauses are rare and have the same diagnostic significance). If the state goes from anything to Idle via Disconnected input, that's detectable because main.rs `return`s immediately on Reconnect action — so we'd need to call `end_session` before returning. Add a `d.on_session_abort()` call in the disconnect/error paths.

Simpler: on entering `run_session()`, if there's an active session in the tracker, abort it (Disconnected). On `any→Idle`, end with outcome derived from peak_state.

**`service.rs:update_diagnostics()`** (after rollup feed):
```rust
d.session_tracker.feed_tick(energy, zcr, speech_band, floor);
```

### Rollup Fields

Add to `RollupEntry`:
```rust
pub session_count: u32,
pub false_session_count: u32,    // SilenceTimeout outcome
pub session_dur_mean_ms: u32,
pub session_dur_max_ms: u32,
```

Add to `RollupAccumulator`:
```rust
session_count: u32,
false_session_count: u32,
session_dur_sum_ms: u64,
session_dur_max_ms: u32,
```

Method: `record_session(duration_ms: u64, outcome: SessionOutcome)` — called by `on_state_change` when a session ends.

### HTTP Endpoint

`GET /sessions` → JSON array of all session records (newest first or oldest first — oldest for consistency with rollups).

### Dashboard

Add session stats to the rollup trend display in `auto_energy.js`:
- "Sessions: X total, Y false (Z%)" from rollup entries
- The `GET /sessions` endpoint exists for manual forensics (`curl`) but doesn't need a dashboard card yet.

---

## Part 2: MFCC Pipeline Stage

### Dependency

Add `realfft = "3"` to `crates/satellite/Cargo.toml`. Pure Rust, wraps `rustfft` for real-signal FFT. No SIMD required — scalar path on ARMv6.

### Config (`config.rs`)

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MfccConfig {
    #[serde(default = "default_mfcc_enabled")]
    pub enabled: bool,
    #[serde(default = "default_n_mfcc")]
    pub n_mfcc: usize,       // default 13
    #[serde(default = "default_n_mels")]
    pub n_mels: usize,       // default 26
    #[serde(default = "default_n_fft")]
    pub n_fft: usize,        // default 512
}
```

Add `pub mfcc: Option<MfccConfig>` to `PipelineConfig`.

### Stage (`pipeline/mfcc.rs`)

**Struct:**
```rust
pub struct MfccStage {
    // Overlap buffer: last (n_fft - frame_size) samples from previous frame
    prev_tail: Vec<f32>,            // 192 samples (512 - 320), pre-allocated

    // Hann window (precomputed at init)
    hann_window: Vec<f32>,          // n_fft values

    // FFT
    fft: Arc<dyn RealToComplex<f32>>,
    fft_input: Vec<f32>,            // n_fft (512)
    fft_output: Vec<Complex<f32>>,  // n_fft/2 + 1 (257)
    fft_scratch: Vec<Complex<f32>>,

    // Power spectrum scratch
    power_spectrum: Vec<f32>,       // n_fft/2 + 1 (257)

    // Mel filterbank (sparse, precomputed)
    mel_filters: Vec<MelFilter>,    // n_mels filters
    mel_energies: Vec<f32>,         // n_mels log-energy outputs

    // DCT-II matrix (precomputed, row-major)
    dct_matrix: Vec<f32>,           // n_mfcc × n_mels
    n_mfcc: usize,
    n_mels: usize,
    n_fft: usize,
    frame_size: usize,              // 320 (from pipeline)

    // Output coefficients (current frame)
    mfcc_coeffs: Vec<f32>,          // n_mfcc values

    // Ring buffer for future wake word matcher
    ring: Vec<Vec<f32>>,            // ring_capacity × n_mfcc
    ring_pos: usize,
    ring_len: usize,

    // Summary stats for diagnostics
    stats_buf: [(StatKey, f64); 2], // spectral_centroid, spectral_flatness
    sample_rate: u32,
}

struct MelFilter {
    start_bin: usize,
    weights: Vec<f32>,
}
```

**Overlapping frames (192-sample overlap):**

The pipeline delivers 320-sample frames at 20ms intervals. The MFCC stage uses a 512-sample FFT window with 192 samples of overlap from the previous frame:

```
prev_tail (192)   current frame (320)
[───────────────] [────────────────────────────]
 ╰─── concat into fft_input (512) ────────────╯
```

On each `process()` call:
1. Copy `prev_tail[0..192]` into `fft_input[0..192]`
2. Convert current 320 i16 samples → f32 into `fft_input[192..512]`
3. Apply Hann window (element-wise multiply, precomputed)
4. Save `fft_input[320..512]` into `prev_tail` for next frame

This gives genuine 31.25 Hz frequency resolution (vs 50 Hz with zero-padding) at the cost of 192 extra f32 copies per frame.

**`process()` flow:**
1. Assemble 512-sample window from overlap + current frame (apply Hann window)
2. Save tail 192 samples for next frame
3. `realfft` → 257 complex bins
4. Compute power spectrum: `|bin|² = re² + im²`
5. Compute spectral centroid and flatness from power spectrum (for stats)
6. Apply mel filterbank (sparse dot products): 26 mel energies
7. Log energy: `ln(max(mel_energy, 1e-10))`
8. DCT-II: `mfcc_coeffs[k] = sum(dct_matrix[k][n] * log_mel[n])` for k=0..12
9. Push coefficients to ring buffer
10. Update stats_buf with centroid and flatness

**Observation-only:** always returns `true`, does not mutate samples.

**`reset()`:** Clear ring buffer and zero prev_tail (new session = new feature context).

### StatKey Additions

```rust
SpectralCentroid,   // "spectral_centroid" — Hz, weighted mean frequency
SpectralFlatness,   // "spectral_flatness" — 0..1, 1=white noise
```

### Rollup Fields

Add to `RollupEntry`:
```rust
pub spectral_centroid_mean: f64,
pub spectral_flatness_mean: f64,
```

Add corresponding sums to `RollupAccumulator`, fed from `extract_stat()` in `update_diagnostics()`.

### StageKind / mod.rs

Add `Mfcc(mfcc::MfccStage)` variant + match arms in all 5 dispatch methods. Insert in observers section of `SimpleRunner::new()`, after ZCR.

### Mel Filterbank Construction

Precomputed at init using HTK mel scale:
- `mel(f) = 2595 * log10(1 + f/700)`
- `f(mel) = 700 * (10^(mel/2595) - 1)`
- Space `n_mels + 2` points evenly on mel scale from 0 to sample_rate/2
- Each filter: triangular window between adjacent center frequencies
- Sparse storage: only non-zero bins per filter

### Hann Window

Precomputed `Vec<f32>` of n_fft (512) values: `w[n] = 0.5 * (1 - cos(2π*n/(N-1)))`. Applied to the assembled 512-sample window (overlap + current frame) before FFT. The window tapers the edges to zero, reducing spectral leakage. With 192-sample overlap, the center of the window covers the current frame's samples with near-unity gain.

---

## File Summary

| File | Change |
|------|--------|
| `crates/satellite/src/diagnostics.rs` | Add `SessionOutcome`, `SessionRecord`, `ActiveSession`, `SessionTracker`, session fields in `RollupEntry`/`RollupAccumulator`, `on_state_change()`, `GET /sessions` endpoint |
| `crates/satellite/src/main.rs` | Call `d.on_state_change()` at transition point (line ~167); abort session on disconnect/error return paths |
| `crates/satellite/src/service.rs` | Call `d.session_tracker.feed_tick()` in `update_diagnostics()` |
| `crates/satellite/src/pipeline/mfcc.rs` | **New file**: `MfccStage` with FFT, mel filterbank, DCT, ring buffer |
| `crates/satellite/src/pipeline/mod.rs` | Add `pub mod mfcc`, `StatKey::{SpectralCentroid, SpectralFlatness}`, `StageKind::Mfcc`, match arms, runner construction |
| `crates/satellite/src/config.rs` | Add `MfccConfig`, `pub mfcc: Option<MfccConfig>` to `PipelineConfig`, defaults |
| `crates/satellite/Cargo.toml` | Add `realfft = "3"` dependency |
| `deploy/satellite.toml` | Add `[pipeline.mfcc]` section |
| `deploy/dashboard/js/renderers/auto_energy.js` | Add session count to trend display from rollup data |

## Verification

1. `cargo test` — all existing + new tests pass
2. `./scripts/deploy.sh` + `./scripts/sync-dashboard.sh`
3. `curl http://10.13.1.51:8585/sessions` — returns `[]`, then records after VAD triggers
4. `curl http://10.13.1.51:8585/rollups` — entries include session_count, spectral stats
5. Dashboard trend shows session counts
6. `work_us` impact: session tracker ~0, MFCC estimated <300µs/frame (measure on Pi)
7. Verify MFCC stage appears in pipeline startup log
