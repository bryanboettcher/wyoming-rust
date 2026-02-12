use super::*;

// Helper to verify transition result
fn assert_transition(
    from: SatelliteState,
    input: SatelliteInput,
    expected_state: SatelliteState,
    expected_actions: Vec<Action>,
) {
    let (new_state, actions) = transition(&from, &input);
    assert_eq!(
        new_state, expected_state,
        "State transition failed for {:?} + {:?}",
        from, input
    );
    assert_eq!(
        actions, expected_actions,
        "Actions mismatch for {:?} + {:?}",
        from, input
    );
}

// Helper for no-op transitions (state stays same, no actions)
fn assert_no_op(state: SatelliteState, input: SatelliteInput) {
    assert_transition(state.clone(), input, state, vec![]);
}

// ============================================================================
// Idle State Tests
// ============================================================================

#[test]
fn idle_gpio_high_starts_streaming() {
    assert_transition(
        SatelliteState::Idle,
        SatelliteInput::GpioHigh,
        SatelliteState::Streaming,
        vec![
            Action::StartCapture,
            Action::SendAudioStart,
            Action::SendStreamingStarted,
        ],
    );
}

#[test]
fn idle_disconnected_stays_idle() {
    assert_transition(
        SatelliteState::Idle,
        SatelliteInput::Disconnected,
        SatelliteState::Idle,
        vec![Action::SetLed(LedState::RedBlink), Action::Reconnect],
    );
}

#[test]
fn idle_ignores_silence_timeout() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::SilenceTimeout);
}

#[test]
fn idle_ignores_playback_complete() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::PlaybackComplete);
}

#[test]
fn idle_ignores_server_run_satellite() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerRunSatellite);
}

#[test]
fn idle_ignores_server_pause_satellite() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerPauseSatellite);
}

#[test]
fn idle_ignores_server_detection() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerDetection);
}

#[test]
fn idle_ignores_server_voice_started() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerVoiceStarted);
}

#[test]
fn idle_ignores_server_tts_start() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerTtsStart);
}

#[test]
fn idle_ignores_server_tts_stop() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerTtsStop);
}

#[test]
fn idle_ignores_server_voice_stopped() {
    assert_no_op(SatelliteState::Idle, SatelliteInput::ServerVoiceStopped);
}

// ============================================================================
// Streaming State Tests
// ============================================================================

#[test]
fn streaming_silence_timeout_returns_to_idle() {
    assert_transition(
        SatelliteState::Streaming,
        SatelliteInput::SilenceTimeout,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::SendAudioStop,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ],
    );
}

#[test]
fn streaming_detection_triggers() {
    assert_transition(
        SatelliteState::Streaming,
        SatelliteInput::ServerDetection,
        SatelliteState::Triggered,
        vec![Action::SetLed(LedState::Cyan)],
    );
}

#[test]
fn streaming_pause_returns_to_idle() {
    assert_transition(
        SatelliteState::Streaming,
        SatelliteInput::ServerPauseSatellite,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::SendAudioStop,
            Action::SendStreamingStopped,
        ],
    );
}

#[test]
fn streaming_disconnected_returns_to_idle() {
    assert_transition(
        SatelliteState::Streaming,
        SatelliteInput::Disconnected,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::StopPlayback,
            Action::SetLed(LedState::RedBlink),
            Action::Reconnect,
        ],
    );
}

#[test]
fn streaming_ignores_gpio_high() {
    assert_no_op(SatelliteState::Streaming, SatelliteInput::GpioHigh);
}

#[test]
fn streaming_ignores_playback_complete() {
    assert_no_op(SatelliteState::Streaming, SatelliteInput::PlaybackComplete);
}

#[test]
fn streaming_ignores_server_run_satellite() {
    assert_no_op(
        SatelliteState::Streaming,
        SatelliteInput::ServerRunSatellite,
    );
}

#[test]
fn streaming_voice_started_skips_to_processing() {
    // Server-side wake word: voice-started arrives without detection first
    assert_transition(
        SatelliteState::Streaming,
        SatelliteInput::ServerVoiceStarted,
        SatelliteState::Processing,
        vec![Action::SetLed(LedState::BluePulse)],
    );
}

#[test]
fn streaming_ignores_server_tts_start() {
    assert_no_op(SatelliteState::Streaming, SatelliteInput::ServerTtsStart);
}

#[test]
fn streaming_ignores_server_tts_stop() {
    assert_no_op(SatelliteState::Streaming, SatelliteInput::ServerTtsStop);
}

#[test]
fn streaming_ignores_server_voice_stopped() {
    assert_no_op(
        SatelliteState::Streaming,
        SatelliteInput::ServerVoiceStopped,
    );
}

// ============================================================================
// Triggered State Tests
// ============================================================================

#[test]
fn triggered_voice_started_enters_processing() {
    assert_transition(
        SatelliteState::Triggered,
        SatelliteInput::ServerVoiceStarted,
        SatelliteState::Processing,
        vec![Action::SetLed(LedState::BluePulse)],
    );
}

#[test]
fn triggered_disconnected_returns_to_idle() {
    assert_transition(
        SatelliteState::Triggered,
        SatelliteInput::Disconnected,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::StopPlayback,
            Action::SetLed(LedState::RedBlink),
            Action::Reconnect,
        ],
    );
}

#[test]
fn triggered_ignores_gpio_high() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::GpioHigh);
}

#[test]
fn triggered_ignores_silence_timeout() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::SilenceTimeout);
}

#[test]
fn triggered_ignores_playback_complete() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::PlaybackComplete);
}

#[test]
fn triggered_ignores_server_run_satellite() {
    assert_no_op(
        SatelliteState::Triggered,
        SatelliteInput::ServerRunSatellite,
    );
}

#[test]
fn triggered_ignores_server_pause_satellite() {
    assert_no_op(
        SatelliteState::Triggered,
        SatelliteInput::ServerPauseSatellite,
    );
}

#[test]
fn triggered_ignores_server_detection() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::ServerDetection);
}

#[test]
fn triggered_ignores_server_tts_start() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::ServerTtsStart);
}

#[test]
fn triggered_ignores_server_tts_stop() {
    assert_no_op(SatelliteState::Triggered, SatelliteInput::ServerTtsStop);
}

#[test]
fn triggered_ignores_server_voice_stopped() {
    assert_no_op(
        SatelliteState::Triggered,
        SatelliteInput::ServerVoiceStopped,
    );
}

// ============================================================================
// Processing State Tests
// ============================================================================

#[test]
fn processing_tts_start_enters_responding() {
    assert_transition(
        SatelliteState::Processing,
        SatelliteInput::ServerTtsStart,
        SatelliteState::Responding,
        vec![
            Action::StopCapture,
            Action::SetLed(LedState::Green),
            Action::StartPlayback,
        ],
    );
}

#[test]
fn processing_disconnected_returns_to_idle() {
    assert_transition(
        SatelliteState::Processing,
        SatelliteInput::Disconnected,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::StopPlayback,
            Action::SetLed(LedState::RedBlink),
            Action::Reconnect,
        ],
    );
}

#[test]
fn processing_ignores_gpio_high() {
    assert_no_op(SatelliteState::Processing, SatelliteInput::GpioHigh);
}

#[test]
fn processing_ignores_silence_timeout() {
    assert_no_op(SatelliteState::Processing, SatelliteInput::SilenceTimeout);
}

#[test]
fn processing_ignores_playback_complete() {
    assert_no_op(SatelliteState::Processing, SatelliteInput::PlaybackComplete);
}

#[test]
fn processing_ignores_server_run_satellite() {
    assert_no_op(
        SatelliteState::Processing,
        SatelliteInput::ServerRunSatellite,
    );
}

#[test]
fn processing_ignores_server_pause_satellite() {
    assert_no_op(
        SatelliteState::Processing,
        SatelliteInput::ServerPauseSatellite,
    );
}

#[test]
fn processing_ignores_server_detection() {
    assert_no_op(SatelliteState::Processing, SatelliteInput::ServerDetection);
}

#[test]
fn processing_ignores_server_voice_started() {
    assert_no_op(
        SatelliteState::Processing,
        SatelliteInput::ServerVoiceStarted,
    );
}

#[test]
fn processing_ignores_server_tts_stop() {
    assert_no_op(SatelliteState::Processing, SatelliteInput::ServerTtsStop);
}

#[test]
fn processing_voice_stopped_returns_to_idle() {
    // Pipeline completed with no TTS response
    assert_transition(
        SatelliteState::Processing,
        SatelliteInput::ServerVoiceStopped,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ],
    );
}

// ============================================================================
// Responding State Tests
// ============================================================================

#[test]
fn responding_tts_stop_returns_to_idle() {
    assert_transition(
        SatelliteState::Responding,
        SatelliteInput::ServerTtsStop,
        SatelliteState::Idle,
        vec![
            Action::StopPlayback,
            Action::SendPlayed,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ],
    );
}

#[test]
fn responding_playback_complete_returns_to_idle() {
    assert_transition(
        SatelliteState::Responding,
        SatelliteInput::PlaybackComplete,
        SatelliteState::Idle,
        vec![
            Action::StopPlayback,
            Action::SendPlayed,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ],
    );
}

#[test]
fn responding_disconnected_returns_to_idle() {
    assert_transition(
        SatelliteState::Responding,
        SatelliteInput::Disconnected,
        SatelliteState::Idle,
        vec![
            Action::StopCapture,
            Action::StopPlayback,
            Action::SetLed(LedState::RedBlink),
            Action::Reconnect,
        ],
    );
}

#[test]
fn responding_ignores_gpio_high() {
    assert_no_op(SatelliteState::Responding, SatelliteInput::GpioHigh);
}

#[test]
fn responding_ignores_silence_timeout() {
    assert_no_op(SatelliteState::Responding, SatelliteInput::SilenceTimeout);
}

#[test]
fn responding_ignores_server_run_satellite() {
    assert_no_op(
        SatelliteState::Responding,
        SatelliteInput::ServerRunSatellite,
    );
}

#[test]
fn responding_ignores_server_pause_satellite() {
    assert_no_op(
        SatelliteState::Responding,
        SatelliteInput::ServerPauseSatellite,
    );
}

#[test]
fn responding_ignores_server_detection() {
    assert_no_op(SatelliteState::Responding, SatelliteInput::ServerDetection);
}

#[test]
fn responding_ignores_server_voice_started() {
    assert_no_op(
        SatelliteState::Responding,
        SatelliteInput::ServerVoiceStarted,
    );
}

#[test]
fn responding_ignores_server_tts_start() {
    assert_no_op(SatelliteState::Responding, SatelliteInput::ServerTtsStart);
}

#[test]
fn responding_voice_stopped_returns_to_idle() {
    // voice-stopped is a valid exit from Responding (redundant with TtsStop)
    assert_transition(
        SatelliteState::Responding,
        SatelliteInput::ServerVoiceStopped,
        SatelliteState::Idle,
        vec![
            Action::StopPlayback,
            Action::SendPlayed,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ],
    );
}

// ============================================================================
// Full Conversation Flow Tests
// ============================================================================

#[test]
fn full_happy_path_conversation() {
    // Start idle
    let mut state = SatelliteState::Idle;

    // GPIO trigger -> Streaming
    let (new_state, actions) = transition(&state, &SatelliteInput::GpioHigh);
    assert_eq!(new_state, SatelliteState::Streaming);
    assert_eq!(
        actions,
        vec![
            Action::StartCapture,
            Action::SendAudioStart,
            Action::SendStreamingStarted,
        ]
    );
    state = new_state;

    // Wake word detection -> Triggered
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerDetection);
    assert_eq!(new_state, SatelliteState::Triggered);
    assert_eq!(actions, vec![Action::SetLed(LedState::Cyan)]);
    state = new_state;

    // Server starts processing -> Processing
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerVoiceStarted);
    assert_eq!(new_state, SatelliteState::Processing);
    assert_eq!(actions, vec![Action::SetLed(LedState::BluePulse)]);
    state = new_state;

    // TTS starts -> Responding
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerTtsStart);
    assert_eq!(new_state, SatelliteState::Responding);
    assert_eq!(
        actions,
        vec![
            Action::StopCapture,
            Action::SetLed(LedState::Green),
            Action::StartPlayback,
        ]
    );
    state = new_state;

    // TTS finishes -> Idle
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerTtsStop);
    assert_eq!(new_state, SatelliteState::Idle);
    assert_eq!(
        actions,
        vec![
            Action::StopPlayback,
            Action::SendPlayed,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ]
    );
}

#[test]
fn interrupted_conversation_pause_during_streaming() {
    let mut state = SatelliteState::Idle;

    // Start streaming
    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    state = new_state;
    assert_eq!(state, SatelliteState::Streaming);

    // Server pauses -> back to Idle
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerPauseSatellite);
    assert_eq!(new_state, SatelliteState::Idle);
    assert_eq!(
        actions,
        vec![
            Action::StopCapture,
            Action::SendAudioStop,
            Action::SendStreamingStopped,
        ]
    );
}

#[test]
fn timeout_during_streaming() {
    let mut state = SatelliteState::Idle;

    // Start streaming
    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    state = new_state;
    assert_eq!(state, SatelliteState::Streaming);

    // Silence timeout -> back to Idle
    let (new_state, actions) = transition(&state, &SatelliteInput::SilenceTimeout);
    assert_eq!(new_state, SatelliteState::Idle);
    assert_eq!(
        actions,
        vec![
            Action::StopCapture,
            Action::SendAudioStop,
            Action::SendStreamingStopped,
            Action::SetLed(LedState::Off),
        ]
    );
}

#[test]
fn disconnection_from_all_states() {
    let states = vec![
        SatelliteState::Idle,
        SatelliteState::Streaming,
        SatelliteState::Triggered,
        SatelliteState::Processing,
        SatelliteState::Responding,
    ];

    for state in states {
        let (new_state, actions) = transition(&state, &SatelliteInput::Disconnected);
        assert_eq!(
            new_state,
            SatelliteState::Idle,
            "Disconnection from {:?} should return to Idle",
            state
        );

        // All disconnections should set RedBlink and Reconnect
        assert!(
            actions.contains(&Action::SetLed(LedState::RedBlink)),
            "Disconnection from {:?} should set RedBlink LED",
            state
        );
        assert!(
            actions.contains(&Action::Reconnect),
            "Disconnection from {:?} should reconnect",
            state
        );

        // Idle just needs LED and reconnect, others need cleanup
        if state == SatelliteState::Idle {
            assert_eq!(actions.len(), 2, "Idle disconnection should have 2 actions");
        } else {
            // All other states should stop capture and playback
            assert!(
                actions.contains(&Action::StopCapture),
                "Disconnection from {:?} should stop capture",
                state
            );
            assert!(
                actions.contains(&Action::StopPlayback),
                "Disconnection from {:?} should stop playback",
                state
            );
            assert_eq!(
                actions.len(),
                4,
                "Non-idle disconnection should have 4 actions"
            );
        }
    }
}

// ============================================================================
// Multi-Cycle and Edge Scenario Tests
// ============================================================================

#[test]
fn double_gpio_high_from_idle() {
    let (state, _) = transition(&SatelliteState::Idle, &SatelliteInput::GpioHigh);
    assert_eq!(state, SatelliteState::Streaming);

    let (state2, actions2) = transition(&state, &SatelliteInput::GpioHigh);
    assert_eq!(state2, SatelliteState::Streaming);
    assert!(actions2.is_empty());
}

#[test]
fn two_complete_conversation_cycles() {
    let inputs_per_cycle = vec![
        SatelliteInput::GpioHigh,
        SatelliteInput::ServerDetection,
        SatelliteInput::ServerVoiceStarted,
        SatelliteInput::ServerTtsStart,
        SatelliteInput::ServerTtsStop,
    ];

    let mut state = SatelliteState::Idle;

    for cycle in 0..2 {
        for input in &inputs_per_cycle {
            let (new_state, _) = transition(&state, input);
            state = new_state;
        }
        assert_eq!(
            state,
            SatelliteState::Idle,
            "Should be back to Idle after cycle {}",
            cycle + 1
        );
    }
}

#[test]
fn recovery_after_disconnection_during_streaming() {
    let mut state = SatelliteState::Idle;

    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    state = new_state;

    let (new_state, actions) = transition(&state, &SatelliteInput::Disconnected);
    state = new_state;
    assert_eq!(state, SatelliteState::Idle);
    assert!(actions.contains(&Action::Reconnect));

    let (new_state, actions) = transition(&state, &SatelliteInput::GpioHigh);
    assert_eq!(new_state, SatelliteState::Streaming);
    assert_eq!(
        actions,
        vec![Action::StartCapture, Action::SendAudioStart, Action::SendStreamingStarted]
    );
}

#[test]
fn recovery_after_disconnection_during_responding() {
    let mut state = SatelliteState::Idle;
    let walk_inputs = vec![
        SatelliteInput::GpioHigh,
        SatelliteInput::ServerDetection,
        SatelliteInput::ServerVoiceStarted,
        SatelliteInput::ServerTtsStart,
    ];
    for input in &walk_inputs {
        let (new_state, _) = transition(&state, input);
        state = new_state;
    }
    assert_eq!(state, SatelliteState::Responding);

    let (new_state, actions) = transition(&state, &SatelliteInput::Disconnected);
    state = new_state;
    assert_eq!(state, SatelliteState::Idle);
    assert!(actions.contains(&Action::StopCapture));
    assert!(actions.contains(&Action::StopPlayback));

    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    assert_eq!(new_state, SatelliteState::Streaming);
}

#[test]
fn action_ordering_streaming_to_idle_on_silence() {
    let (_, actions) = transition(&SatelliteState::Streaming, &SatelliteInput::SilenceTimeout);

    let stop_capture_pos = actions.iter().position(|a| a == &Action::StopCapture).unwrap();
    let send_audio_stop_pos = actions.iter().position(|a| a == &Action::SendAudioStop).unwrap();
    let send_streaming_stopped_pos = actions
        .iter()
        .position(|a| a == &Action::SendStreamingStopped)
        .unwrap();

    assert!(stop_capture_pos < send_audio_stop_pos);
    assert!(send_audio_stop_pos < send_streaming_stopped_pos);
}

#[test]
fn action_ordering_processing_to_responding() {
    let (_, actions) = transition(&SatelliteState::Processing, &SatelliteInput::ServerTtsStart);

    let stop_capture_pos = actions.iter().position(|a| a == &Action::StopCapture).unwrap();
    let start_playback_pos = actions.iter().position(|a| a == &Action::StartPlayback).unwrap();

    assert!(stop_capture_pos < start_playback_pos);
}

#[test]
fn transition_is_pure_function() {
    let state = SatelliteState::Idle;
    let input = SatelliteInput::GpioHigh;

    let (state1, actions1) = transition(&state, &input);
    let (state2, actions2) = transition(&state, &input);

    assert_eq!(state1, state2);
    assert_eq!(actions1, actions2);
}

#[test]
fn server_side_wake_word_skips_triggered() {
    // When the server detects the wake word itself, it sends voice-started
    // directly without a detection event.
    let mut state = SatelliteState::Idle;

    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    state = new_state;
    assert_eq!(state, SatelliteState::Streaming);

    // Server sends voice-started directly (no detection)
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerVoiceStarted);
    state = new_state;
    assert_eq!(state, SatelliteState::Processing);
    assert_eq!(actions, vec![Action::SetLed(LedState::BluePulse)]);

    // Continue normally
    let (new_state, _) = transition(&state, &SatelliteInput::ServerTtsStart);
    state = new_state;
    assert_eq!(state, SatelliteState::Responding);

    let (new_state, _) = transition(&state, &SatelliteInput::ServerTtsStop);
    assert_eq!(new_state, SatelliteState::Idle);
}

#[test]
fn pipeline_with_no_tts_response() {
    // Intent handler responds with "OK" but no TTS audio — server sends
    // voice-stopped without audio-start/audio-stop.
    let mut state = SatelliteState::Idle;

    let (new_state, _) = transition(&state, &SatelliteInput::GpioHigh);
    state = new_state;
    let (new_state, _) = transition(&state, &SatelliteInput::ServerDetection);
    state = new_state;
    let (new_state, _) = transition(&state, &SatelliteInput::ServerVoiceStarted);
    state = new_state;
    assert_eq!(state, SatelliteState::Processing);

    // No TTS — server sends voice-stopped directly
    let (new_state, actions) = transition(&state, &SatelliteInput::ServerVoiceStopped);
    assert_eq!(new_state, SatelliteState::Idle);
    assert!(actions.contains(&Action::StopCapture));
    assert!(actions.contains(&Action::SendStreamingStopped));
    assert!(actions.contains(&Action::SetLed(LedState::Off)));
}

#[test]
fn voice_stopped_exits_responding() {
    // voice-stopped during Responding is a valid exit (redundant with TtsStop)
    let mut state = SatelliteState::Idle;
    let walk = vec![
        SatelliteInput::GpioHigh,
        SatelliteInput::ServerDetection,
        SatelliteInput::ServerVoiceStarted,
        SatelliteInput::ServerTtsStart,
    ];
    for input in &walk {
        let (new_state, _) = transition(&state, input);
        state = new_state;
    }
    assert_eq!(state, SatelliteState::Responding);

    let (new_state, actions) = transition(&state, &SatelliteInput::ServerVoiceStopped);
    assert_eq!(new_state, SatelliteState::Idle);
    assert!(actions.contains(&Action::StopPlayback));
    assert!(actions.contains(&Action::SendPlayed));
}

#[test]
fn playback_complete_only_matters_in_responding() {
    let non_responding_states = vec![
        SatelliteState::Idle,
        SatelliteState::Streaming,
        SatelliteState::Triggered,
        SatelliteState::Processing,
    ];

    for state in non_responding_states {
        let (new_state, actions) = transition(&state, &SatelliteInput::PlaybackComplete);
        assert_eq!(new_state, state);
        assert!(actions.is_empty());
    }

    let (new_state, actions) = transition(
        &SatelliteState::Responding,
        &SatelliteInput::PlaybackComplete,
    );
    assert_eq!(new_state, SatelliteState::Idle);
    assert!(!actions.is_empty());
}
