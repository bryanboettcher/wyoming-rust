use std::io::Write;
use std::sync::mpsc;

use crate::config::{ConsoleEffect, StateEffects};
use crate::state::FeedbackState;

/// Console feedback provider worker.
///
/// Prints configured templates to stdout or stderr on state changes.
/// Unconfigured states produce no output. Deduplicates consecutive identical states.
pub fn worker(
    rx: mpsc::Receiver<FeedbackState>,
    output: &str,
    states: &StateEffects<ConsoleEffect>,
) {
    let use_stderr = output == "stderr";
    let mut current = None;

    while let Ok(state) = rx.recv() {
        if Some(state) == current {
            continue;
        }
        current = Some(state);

        if let Some(effect) = states.get(state) {
            if use_stderr {
                let _ = writeln!(std::io::stderr(), "{}", effect.template);
            } else {
                let _ = writeln!(std::io::stdout(), "{}", effect.template);
            }
        }
    }
    log::debug!("Console feedback worker shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn worker_exits_on_channel_close() {
        let (tx, rx) = mpsc::channel();
        let states = StateEffects::default();
        drop(tx);
        // Worker should return immediately when channel is closed
        worker(rx, "stdout", &states);
    }

    #[test]
    fn worker_handles_configured_and_unconfigured_states() {
        let (tx, rx) = mpsc::channel();
        let states = StateEffects {
            idle: Some(ConsoleEffect {
                template: "IDLE".into(),
            }),
            listening: None, // unconfigured — should produce no output
            detected: Some(ConsoleEffect {
                template: "DETECTED".into(),
            }),
            processing: None,
            speaking: None,
            error: None,
        };

        // Send states then close channel
        tx.send(FeedbackState::Idle).unwrap();
        tx.send(FeedbackState::Listening).unwrap();
        tx.send(FeedbackState::Detected).unwrap();
        drop(tx);

        // Worker processes all messages and exits cleanly.
        // (We can't easily capture stdout in tests, but we verify no panics.)
        worker(rx, "stdout", &states);
    }

    #[test]
    fn worker_deduplicates_consecutive_states() {
        let (tx, rx) = mpsc::channel();
        let states = StateEffects {
            idle: Some(ConsoleEffect {
                template: "IDLE".into(),
            }),
            listening: None,
            detected: None,
            processing: None,
            speaking: None,
            error: None,
        };

        // Send same state multiple times
        tx.send(FeedbackState::Idle).unwrap();
        tx.send(FeedbackState::Idle).unwrap();
        tx.send(FeedbackState::Idle).unwrap();
        drop(tx);

        // Should only print once, no panics
        worker(rx, "stderr", &states);
    }
}
