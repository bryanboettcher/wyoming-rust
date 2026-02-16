#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatelliteState {
    /// No sound detected. No audio capture, no streaming.
    Idle,
    /// Schmitt trigger fired. Capturing mic audio, streaming to server.
    Streaming,
    /// Wake word detected by server. Still streaming.
    Triggered,
    /// Server processing: STT -> intent -> TTS.
    Processing,
    /// Playing TTS audio through speaker. Mic muted.
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
    SetFeedback(FeedbackState),
    Reconnect,
}

/// Semantic feedback state. Describes *what happened*, not how to present it.
/// Feedback providers map these to hardware-specific output (colors, tones, etc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackState {
    /// Satellite ready, waiting for input.
    Idle,
    /// Mic active, streaming audio to server.
    Listening,
    /// Wake word recognized by server.
    Detected,
    /// Server processing (STT -> intent -> TTS).
    Processing,
    /// TTS playback in progress.
    Speaking,
    /// Disconnected or error condition.
    Error,
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
        actions.push(Action::SetFeedback(FeedbackState::Error));
        actions.push(Action::Reconnect);
        return (Idle, actions);
    }

    match (state, input) {
        // ── IDLE ──────────────────────────────────────────
        (Idle, GpioHigh) => (
            Streaming,
            vec![Action::SetFeedback(FeedbackState::Listening), Action::StartCapture, Action::SendAudioStart, Action::SendStreamingStarted],
        ),

        // ── STREAMING ─────────────────────────────────────
        (Streaming, SilenceTimeout) => (
            Idle,
            vec![Action::StopCapture, Action::SendAudioStop, Action::SendStreamingStopped, Action::SetFeedback(FeedbackState::Idle)],
        ),
        (Streaming, ServerDetection) => (
            Triggered,
            vec![Action::SetFeedback(FeedbackState::Detected)],
        ),
        // Server-side wake word: server may send voice-started without detection first
        (Streaming, ServerVoiceStarted) => (
            Processing,
            vec![Action::SetFeedback(FeedbackState::Processing)],
        ),
        (Streaming, ServerPauseSatellite) => (
            Idle,
            vec![Action::StopCapture, Action::SendAudioStop, Action::SendStreamingStopped],
        ),

        // ── TRIGGERED ─────────────────────────────────────
        (Triggered, ServerVoiceStarted) => (
            Processing,
            vec![Action::SetFeedback(FeedbackState::Processing)],
        ),

        // ── PROCESSING ────────────────────────────────────
        (Processing, ServerTtsStart) => (
            Responding,
            vec![Action::StopCapture, Action::SetFeedback(FeedbackState::Speaking), Action::StartPlayback],
        ),
        // Pipeline completed with no TTS response (e.g. "OK" with no audio)
        (Processing, ServerVoiceStopped) => (
            Idle,
            vec![Action::StopCapture, Action::SendStreamingStopped, Action::SetFeedback(FeedbackState::Idle)],
        ),

        // ── RESPONDING ────────────────────────────────────
        (Responding, ServerTtsStop) | (Responding, ServerVoiceStopped) | (Responding, PlaybackComplete) => (
            Idle,
            vec![Action::StopPlayback, Action::SendPlayed, Action::SendStreamingStopped, Action::SetFeedback(FeedbackState::Idle)],
        ),

        // Default: no transition, no actions
        _ => (state.clone(), vec![]),
    }
}

#[cfg(test)]
mod tests;
