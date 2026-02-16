use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use rppal::gpio::{Gpio, OutputPin};

use crate::config::{PwmEffect, PwmEffectType, StateEffects};
use crate::state::FeedbackState;

/// Animation step interval — 50Hz update rate for smooth effects.
const STEP_MS: u64 = 20;

/// PWM feedback provider worker.
///
/// Drives a single GPIO pin with software PWM. Covers both LEDs (high-frequency
/// PWM, modulate duty cycle for brightness) and buzzers (audible-frequency PWM).
///
/// Static effects (`off`, `solid`) block on `rx.recv()` — zero CPU.
/// Animated effects (`breathe`, `blink`, `pulse`) loop with `try_recv()`.
///
/// GPIO is initialized inside the worker thread. If init fails, the provider
/// exits gracefully — other providers are unaffected.
pub fn worker(
    rx: mpsc::Receiver<FeedbackState>,
    pin_num: u32,
    states: &StateEffects<PwmEffect>,
) {
    let gpio = match Gpio::new() {
        Ok(g) => g,
        Err(e) => {
            log::error!("PWM feedback: failed to initialize GPIO: {}", e);
            return;
        }
    };
    let mut pin = match gpio.get(pin_num as u8) {
        Ok(p) => p.into_output_low(),
        Err(e) => {
            log::error!("PWM feedback: failed to get GPIO pin {}: {}", pin_num, e);
            return;
        }
    };

    let mut current_state = FeedbackState::Idle;

    // Apply initial state
    if let Some(effect) = states.get(current_state) {
        run_effect(&rx, &mut pin, effect, &mut current_state);
    } else {
        // No config for initial state — wait for next
        match rx.recv() {
            Ok(s) => current_state = s,
            Err(_) => {
                pin.set_low();
                return;
            }
        }
    }

    loop {
        if let Some(effect) = states.get(current_state) {
            if !run_effect(&rx, &mut pin, effect, &mut current_state) {
                break;
            }
        } else {
            // No config for this state — pin off, wait for next state
            pin.clear_pwm().ok();
            pin.set_low();
            match rx.recv() {
                Ok(s) => current_state = s,
                Err(_) => break,
            }
        }
    }

    pin.clear_pwm().ok();
    pin.set_low();
    log::debug!("PWM feedback worker shutting down (pin {})", pin_num);
}

/// Run an effect until a new state arrives. Returns false if channel is closed.
fn run_effect(
    rx: &mpsc::Receiver<FeedbackState>,
    pin: &mut OutputPin,
    effect: &PwmEffect,
    current_state: &mut FeedbackState,
) -> bool {
    match effect.effect {
        PwmEffectType::Off => {
            pin.clear_pwm().ok();
            pin.set_low();
            match rx.recv() {
                Ok(s) => {
                    *current_state = s;
                    true
                }
                Err(_) => false,
            }
        }

        PwmEffectType::Solid => {
            let _ = pin.set_pwm_frequency(effect.frequency, effect.duty);
            match rx.recv() {
                Ok(s) => {
                    pin.clear_pwm().ok();
                    *current_state = s;
                    true
                }
                Err(_) => false,
            }
        }

        PwmEffectType::Breathe => {
            let period_ms = effect.period_ms.unwrap_or(2000);
            let steps = period_ms / STEP_MS;
            let mut step: u64 = 0;

            loop {
                // Sinusoidal duty ramp: 0 → duty → 0
                let phase = (step as f64 / steps as f64) * std::f64::consts::TAU;
                let duty = effect.duty * (0.5 - 0.5 * phase.cos());
                let _ = pin.set_pwm_frequency(effect.frequency, duty);

                std::thread::sleep(Duration::from_millis(STEP_MS));
                step = (step + 1) % steps;

                match rx.try_recv() {
                    Ok(s) => {
                        pin.clear_pwm().ok();
                        *current_state = s;
                        return true;
                    }
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => return false,
                }
            }
        }

        PwmEffectType::Blink => {
            let period_ms = effect.period_ms.unwrap_or(500);
            let step_count = (period_ms / 2) / STEP_MS;

            loop {
                // On phase
                let _ = pin.set_pwm_frequency(effect.frequency, effect.duty);
                for _ in 0..step_count {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            pin.clear_pwm().ok();
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }

                // Off phase
                pin.clear_pwm().ok();
                pin.set_low();
                for _ in 0..step_count {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }
            }
        }

        PwmEffectType::Pulse => {
            let count = effect.count.unwrap_or(2);
            let pulse_ms = effect.pulse_ms.unwrap_or(150);
            let pulse_steps = pulse_ms / STEP_MS;

            for _ in 0..count {
                // On
                let _ = pin.set_pwm_frequency(effect.frequency, effect.duty);
                for _ in 0..pulse_steps {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            pin.clear_pwm().ok();
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }
                // Off
                pin.clear_pwm().ok();
                pin.set_low();
                for _ in 0..pulse_steps {
                    std::thread::sleep(Duration::from_millis(STEP_MS));
                    match rx.try_recv() {
                        Ok(s) => {
                            *current_state = s;
                            return true;
                        }
                        Err(TryRecvError::Empty) => continue,
                        Err(TryRecvError::Disconnected) => return false,
                    }
                }
            }

            // After pulses: pin off, wait for next state
            match rx.recv() {
                Ok(s) => {
                    *current_state = s;
                    true
                }
                Err(_) => false,
            }
        }
    }
}

// Note: actual rppal GPIO calls can't run in CI (no /dev/gpiomem).
// The effect logic is tested indirectly through integration tests on real hardware.
// The worker function signature and state machine are verified by the fanout tests
// in feedback/mod.rs and the config parsing tests in config.rs.
