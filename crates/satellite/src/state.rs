#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatelliteState {
    /// No sound detected. No audio capture, no streaming.
    Idle,
    /// Schmitt trigger fired. Capturing mic audio, streaming to server.
    Streaming,
    /// Wake word detected by server. LED cyan. Still streaming.
    Triggered,
    /// Server processing: STT -> intent -> TTS. LED blue pulse.
    Processing,
    /// Playing TTS audio through speaker. LED green. Mic muted.
    Responding,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SatelliteInput {
    // Hardware
    GpioHigh,
    SilenceTimeout,
    #[allow(dead_code)] // Used when real ALSA playback reports completion
    PlaybackComplete,

    // Server messages
    ServerRunSatellite,
    ServerPauseSatellite,
    ServerDetection,
    ServerVoiceStarted,
    ServerTtsStart,
    ServerTtsStop,
    ServerVoiceStopped,

    // Connection
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    StartCapture,
    StopCapture,
    SendAudioStart,
    SendAudioStop,
    SendStreamingStarted,
    SendStreamingStopped,
    StartPlayback,
    StopPlayback,
    SendPlayed,
    SetLed(LedState),
    Reconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedState {
    Off,
    DimWhite,
    Cyan,
    BluePulse,
    Green,
    RedBlink,
}

/// Pure function: (current_state, input) -> (new_state, actions[])
/// No I/O, no side effects. Fully testable.
pub fn transition(state: &SatelliteState, input: &SatelliteInput) -> (SatelliteState, Vec<Action>) {
    use SatelliteState::*;
    use SatelliteInput::*;

    // Disconnection from any state always goes to Idle with cleanup
    if matches!(input, Disconnected) {
        let mut actions = vec![];
        if !matches!(state, Idle) {
            actions.push(Action::StopCapture);
            actions.push(Action::StopPlayback);
        }
        actions.push(Action::SetLed(LedState::RedBlink));
        actions.push(Action::Reconnect);
        return (Idle, actions);
    }

    match (state, input) {
        // ── IDLE ──────────────────────────────────────────
        (Idle, GpioHigh) => (
            Streaming,
            vec![Action::StartCapture, Action::SendAudioStart, Action::SendStreamingStarted],
        ),

        // ── STREAMING ─────────────────────────────────────
        (Streaming, SilenceTimeout) => (
            Idle,
            vec![Action::StopCapture, Action::SendAudioStop, Action::SendStreamingStopped, Action::SetLed(LedState::Off)],
        ),
        (Streaming, ServerDetection) => (
            Triggered,
            vec![Action::SetLed(LedState::Cyan)],
        ),
        // Server-side wake word: server may send voice-started without detection first
        (Streaming, ServerVoiceStarted) => (
            Processing,
            vec![Action::SetLed(LedState::BluePulse)],
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
        (Processing, ServerTtsStart) => (
            Responding,
            vec![Action::StopCapture, Action::SetLed(LedState::Green), Action::StartPlayback],
        ),
        // Pipeline completed with no TTS response (e.g. "OK" with no audio)
        (Processing, ServerVoiceStopped) => (
            Idle,
            vec![Action::StopCapture, Action::SendStreamingStopped, Action::SetLed(LedState::Off)],
        ),

        // ── RESPONDING ────────────────────────────────────
        (Responding, ServerTtsStop) | (Responding, ServerVoiceStopped) | (Responding, PlaybackComplete) => (
            Idle,
            vec![Action::StopPlayback, Action::SendPlayed, Action::SendStreamingStopped, Action::SetLed(LedState::Off)],
        ),

        // Default: no transition, no actions
        _ => (state.clone(), vec![]),
    }
}

#[cfg(test)]
mod tests;
