/**
 * Statistics utilities for dashboard diagnostics.
 *
 * Real-time tier: Raw 60s buffer (3000 samples) → percentiles via computeStats()
 * Trend tier: Server-side minute rollups fetched from /api/rollups
 */

/**
 * Compute descriptive statistics from a numeric array.
 * Sorts a copy internally for percentile computation.
 *
 * @param {number[]} samples
 * @returns {{ min: number, max: number, mean: number, p50: number, p95: number, p99: number, stddev: number, count: number }}
 */
export function computeStats(samples) {
  const n = samples.length;
  if (n === 0) return { min: 0, max: 0, mean: 0, p50: 0, p95: 0, p99: 0, stddev: 0, count: 0 };

  const sorted = samples.slice().sort((a, b) => a - b);
  const sum = sorted.reduce((a, b) => a + b, 0);
  const mean = sum / n;

  let variance = 0;
  for (let i = 0; i < n; i++) {
    const d = sorted[i] - mean;
    variance += d * d;
  }
  variance /= n;

  return {
    min: sorted[0],
    max: sorted[n - 1],
    mean,
    p50: sorted[Math.floor(n * 0.50)],
    p95: sorted[Math.floor(n * 0.95)],
    p99: sorted[Math.min(Math.floor(n * 0.99), n - 1)],
    stddev: Math.sqrt(variance),
    count: n,
  };
}

/**
 * Tracks trigger onset/offset events from the 50Hz triggered stat.
 * No unbounded storage — just running counters and timestamps.
 */
export class TriggerTracker {
  constructor() {
    this.reset();
  }

  reset() {
    this._wasTriggered = false;
    this._totalTriggers = 0;
    this._totalFrames = 0;
    this._triggeredFrames = 0;
    this._onsetTime = null;
    this._durationSum = 0;
    this._maxDuration = 0;
    this._lastOnsetTime = null;
    // Snapshot accumulators (reset each minute)
    this._minuteTriggers = 0;
    this._minuteFrames = 0;
    this._minuteTriggeredFrames = 0;
  }

  /**
   * Feed one tick's triggered state.
   * @param {boolean} triggered
   */
  push(triggered) {
    const now = Date.now();
    this._totalFrames++;
    this._minuteFrames++;

    if (triggered) {
      this._triggeredFrames++;
      this._minuteTriggeredFrames++;
    }

    // Onset: 0→1
    if (triggered && !this._wasTriggered) {
      this._totalTriggers++;
      this._minuteTriggers++;
      this._onsetTime = now;
      this._lastOnsetTime = now;
    }

    // Offset: 1→0
    if (!triggered && this._wasTriggered && this._onsetTime != null) {
      const dur = now - this._onsetTime;
      this._durationSum += dur;
      if (dur > this._maxDuration) this._maxDuration = dur;
      this._onsetTime = null;
    }

    this._wasTriggered = triggered;
  }

  /** Real-time stats for display. */
  get stats() {
    const completed = this._totalTriggers - (this._onsetTime != null ? 1 : 0);
    return {
      totalTriggers: this._totalTriggers,
      avgDurationMs: completed > 0 ? this._durationSum / completed : 0,
      maxDurationMs: this._maxDuration,
      lastTriggerAgo: this._lastOnsetTime != null ? Date.now() - this._lastOnsetTime : null,
      triggerPercent: this._totalFrames > 0 ? (this._triggeredFrames / this._totalFrames) * 100 : 0,
      totalFrames: this._totalFrames,
    };
  }

  /**
   * Extract minute-level snapshot and reset minute accumulators.
   * Called once per rollup interval.
   */
  minuteSnapshot() {
    const snap = {
      triggerCount: this._minuteTriggers,
      triggerPercent: this._minuteFrames > 0
        ? (this._minuteTriggeredFrames / this._minuteFrames) * 100
        : 0,
      triggeredFrames: this._minuteTriggeredFrames,
      totalFrames: this._minuteFrames,
    };
    this._minuteTriggers = 0;
    this._minuteFrames = 0;
    this._minuteTriggeredFrames = 0;
    return snap;
  }
}
