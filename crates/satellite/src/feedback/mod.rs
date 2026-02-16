pub mod console;
pub mod neopixel;
pub mod pwm;

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crate::state::FeedbackState;

/// Abstraction for satellite user feedback.
///
/// Implementations receive semantic state updates from the state machine
/// and translate them to hardware-specific output (LEDs, buzzers, etc).
///
/// `update()` MUST return immediately. Providers that need sustained output
/// (tones, animations) must manage their own timing internally.
pub trait Feedback: Send {
    fn update(&mut self, state: FeedbackState);
    fn shutdown(&mut self);
}

/// Fans out feedback state updates to multiple provider worker threads.
///
/// Each provider runs on its own thread, receiving state updates via a channel.
/// `update()` is always non-blocking — it just posts to each provider's channel.
pub struct FanoutFeedback {
    senders: Vec<Sender<FeedbackState>>,
    workers: Vec<JoinHandle<()>>,
}

impl FanoutFeedback {
    pub fn new() -> Self {
        Self {
            senders: Vec::new(),
            workers: Vec::new(),
        }
    }

    /// Spawn a provider worker thread. The `worker_fn` runs in a loop, receiving
    /// `FeedbackState` values from a channel. When the channel is closed (on
    /// shutdown), the function should return.
    pub fn add_provider<F>(&mut self, name: &str, worker_fn: F)
    where
        F: FnOnce(mpsc::Receiver<FeedbackState>) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let thread_name = format!("feedback-{}", name);
        let handle = thread::Builder::new()
            .name(thread_name.clone())
            .stack_size(64 * 1024) // 64KB — providers use minimal stack
            .spawn(move || worker_fn(rx))
            .unwrap_or_else(|e| panic!("failed to spawn {}: {}", thread_name, e));

        self.senders.push(tx);
        self.workers.push(handle);
    }
}

impl Feedback for FanoutFeedback {
    fn update(&mut self, state: FeedbackState) {
        for tx in &self.senders {
            // Ignore send errors — the worker may have panicked, but we
            // don't want to take down the satellite for a feedback failure.
            let _ = tx.send(state);
        }
    }

    fn shutdown(&mut self) {
        // Drop all senders so worker threads see channel-closed and exit.
        self.senders.clear();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Logs feedback state changes. Replacement for the old StubLed.
/// Runs as a provider worker thread within FanoutFeedback.
pub fn logging_worker(rx: mpsc::Receiver<FeedbackState>) {
    let mut current = FeedbackState::Idle;
    while let Ok(state) = rx.recv() {
        if state != current {
            log::info!("Feedback: {:?} -> {:?}", current, state);
            current = state;
        }
    }
    log::debug!("Logging feedback worker shutting down");
}

/// Captures feedback state changes for test assertions.
pub struct RecordingFeedback {
    pub states: Vec<FeedbackState>,
}

impl RecordingFeedback {
    pub fn new() -> Self {
        Self { states: Vec::new() }
    }
}

impl Feedback for RecordingFeedback {
    fn update(&mut self, state: FeedbackState) {
        self.states.push(state);
    }

    fn shutdown(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_feedback_captures_states() {
        let mut fb = RecordingFeedback::new();
        fb.update(FeedbackState::Idle);
        fb.update(FeedbackState::Listening);
        fb.update(FeedbackState::Detected);

        assert_eq!(fb.states.len(), 3);
        assert_eq!(fb.states[0], FeedbackState::Idle);
        assert_eq!(fb.states[1], FeedbackState::Listening);
        assert_eq!(fb.states[2], FeedbackState::Detected);
    }

    #[test]
    fn fanout_delivers_to_all_providers() {
        use std::sync::{Arc, Mutex};

        let received = Arc::new(Mutex::new(Vec::new()));

        let mut fanout = FanoutFeedback::new();

        // Add two simple recording workers
        for _ in 0..2 {
            let captured = Arc::clone(&received);
            fanout.add_provider("test", move |rx| {
                while let Ok(state) = rx.recv() {
                    captured.lock().unwrap().push(state);
                }
            });
        }

        fanout.update(FeedbackState::Listening);
        fanout.update(FeedbackState::Detected);
        fanout.shutdown();

        let states = received.lock().unwrap();
        // 2 providers * 2 updates = 4 entries
        assert_eq!(states.len(), 4);
        assert_eq!(
            states.iter().filter(|s| **s == FeedbackState::Listening).count(),
            2
        );
        assert_eq!(
            states.iter().filter(|s| **s == FeedbackState::Detected).count(),
            2
        );
    }

    #[test]
    fn fanout_survives_dead_worker() {
        let mut fanout = FanoutFeedback::new();

        // Worker that immediately exits
        fanout.add_provider("dying", |_rx| {
            // Drop rx immediately — simulates a panicking provider
        });

        // Should not panic even though the worker is dead
        fanout.update(FeedbackState::Error);
        fanout.shutdown();
    }

    #[test]
    fn empty_fanout_is_valid() {
        let mut fanout = FanoutFeedback::new();
        fanout.update(FeedbackState::Idle);
        fanout.shutdown();
    }

    #[test]
    fn fanout_survives_panicking_worker() {
        let mut fanout = FanoutFeedback::new();

        fanout.add_provider("panicker", |rx| {
            let _ = rx.recv(); // receive one message, then panic
            panic!("simulated provider crash");
        });

        // First update is received, then the worker panics
        fanout.update(FeedbackState::Listening);

        // Give the worker time to panic
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Subsequent updates should not block or panic the main thread
        fanout.update(FeedbackState::Detected);
        fanout.update(FeedbackState::Processing);

        // Shutdown should handle the panicked thread gracefully
        fanout.shutdown();
    }

    #[test]
    fn update_does_not_block_on_slow_worker() {
        use std::time::{Duration, Instant};

        let mut fanout = FanoutFeedback::new();

        fanout.add_provider("slow", |rx| {
            while let Ok(_state) = rx.recv() {
                // Simulate a slow provider (e.g. buzzer generating a tone)
                std::thread::sleep(Duration::from_millis(200));
            }
        });

        let start = Instant::now();
        for _ in 0..5 {
            fanout.update(FeedbackState::Detected);
        }
        let elapsed = start.elapsed();

        // 5 sends via channel should complete in well under 50ms,
        // even though the worker takes 200ms per message.
        // mpsc is unbounded so sends never block.
        assert!(
            elapsed < Duration::from_millis(50),
            "update() blocked for {:?} — should be non-blocking",
            elapsed
        );

        fanout.shutdown();
    }

    #[test]
    fn updates_arrive_in_order_per_provider() {
        use std::sync::{Arc, Mutex};

        let received = Arc::new(Mutex::new(Vec::new()));
        let mut fanout = FanoutFeedback::new();

        let captured = Arc::clone(&received);
        fanout.add_provider("ordered", move |rx| {
            while let Ok(state) = rx.recv() {
                captured.lock().unwrap().push(state);
            }
        });

        let sequence = [
            FeedbackState::Idle,
            FeedbackState::Listening,
            FeedbackState::Detected,
            FeedbackState::Processing,
            FeedbackState::Speaking,
            FeedbackState::Idle,
        ];
        for state in &sequence {
            fanout.update(*state);
        }
        fanout.shutdown();

        let states = received.lock().unwrap();
        assert_eq!(states.as_slice(), &sequence);
    }

    #[test]
    fn shutdown_while_worker_is_busy() {
        use std::sync::{Arc, Mutex};

        let received = Arc::new(Mutex::new(Vec::new()));
        let mut fanout = FanoutFeedback::new();

        let captured = Arc::clone(&received);
        fanout.add_provider("busy", move |rx| {
            while let Ok(state) = rx.recv() {
                // Simulate slow processing
                std::thread::sleep(std::time::Duration::from_millis(100));
                captured.lock().unwrap().push(state);
            }
        });

        // Queue up several updates
        fanout.update(FeedbackState::Listening);
        fanout.update(FeedbackState::Detected);
        fanout.update(FeedbackState::Processing);

        // Shutdown while the worker is likely still processing the first message.
        // Dropping the sender causes the channel to close — the worker will
        // finish its current recv() but the next recv() returns Err.
        fanout.shutdown();

        // The worker processed at least the first message, possibly more.
        let states = received.lock().unwrap();
        assert!(
            !states.is_empty(),
            "Worker should have processed at least one message before shutdown"
        );
    }

    #[test]
    fn healthy_provider_unaffected_by_dead_sibling() {
        use std::sync::{Arc, Mutex};

        let received = Arc::new(Mutex::new(Vec::new()));
        let mut fanout = FanoutFeedback::new();

        // Provider 1: dies immediately
        fanout.add_provider("dead", |_rx| {});

        // Provider 2: healthy, records everything
        let captured = Arc::clone(&received);
        fanout.add_provider("healthy", move |rx| {
            while let Ok(state) = rx.recv() {
                captured.lock().unwrap().push(state);
            }
        });

        // Give the dead worker time to exit
        std::thread::sleep(std::time::Duration::from_millis(50));

        fanout.update(FeedbackState::Listening);
        fanout.update(FeedbackState::Detected);
        fanout.shutdown();

        let states = received.lock().unwrap();
        assert_eq!(states.len(), 2);
        assert_eq!(states[0], FeedbackState::Listening);
        assert_eq!(states[1], FeedbackState::Detected);
    }
}
