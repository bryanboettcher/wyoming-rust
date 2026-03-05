/**
 * Specialized renderer for auto_energy VAD stages.
 *
 * Displays energy level, noise floor, attack/sustain thresholds with
 * tuning controls, triggered state, phase indicator, energy sparkline,
 * real-time statistics (60s), and 24h minute-level trend rollups.
 */

import { drawSparkline } from '../sparkline.js';
import { MAX_ENERGY_SAMPLES, SPARKLINE_VISIBLE, snapshotData } from '../state.js';
import { computeStats, TriggerTracker } from '../stats.js';

/** Ticks between real-time stats DOM updates (~1Hz at 50Hz tick rate). */
const STATS_UPDATE_INTERVAL = 50;

function flash(el, cls) {
  el.classList.remove('flash-ok', 'flash-err');
  void el.offsetWidth;
  el.classList.add(cls);
}

function fmtDuration(ms) {
  if (ms == null) return '\u2014';
  if (ms < 1000) return Math.round(ms) + 'ms';
  return (ms / 1000).toFixed(1) + 's';
}

function fmtAgo(ms) {
  if (ms == null) return '\u2014';
  if (ms < 1000) return '<1s ago';
  if (ms < 60000) return Math.floor(ms / 1000) + 's ago';
  return Math.floor(ms / 60000) + 'm ago';
}

function fmtTime(date) {
  if (!date) return '?';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export class AutoEnergyRenderer {
  constructor() {
    this.card = null;
    this.els = {};
    this.energyHistory = [];
    this.noiseFloorHistory = [];
    this.attackThreshold = null;
    this.sustainThreshold = null;
    this.sparklineDirty = false;
    this.triggerTracker = new TriggerTracker();
    this._serverRollups = [];
    this.statsCounter = 0;
  }

  /**
   * Create the card DOM element.
   * @param {string} name - Stage name
   * @returns {HTMLElement}
   */
  create(name) {
    const card = document.createElement('div');
    card.className = 'stage-card';

    const title = this._prettyName(name);

    card.innerHTML =
      '<div class="card-title">' + this._escapeHtml(title) + '</div>' +
      '<div class="row">' +
        '<span class="label">Energy</span>' +
        '<span class="value" data-field="energy">\u2014</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Noise Floor</span>' +
        '<span class="value" data-field="noise_floor">\u2014</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Attack Threshold</span>' +
        '<span class="value" data-field="attack_threshold">\u2014</span>' +
        '<span class="threshold-control">' +
          '<input type="number" data-field="attack_input" min="0.1" max="100" step="0.1" placeholder="ratio">' +
          '<button data-field="attack_btn">Set</button>' +
        '</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Sustain Threshold</span>' +
        '<span class="value" data-field="sustain_threshold">\u2014</span>' +
        '<span class="threshold-control">' +
          '<input type="number" data-field="sustain_input" min="0.1" max="100" step="0.1" placeholder="ratio">' +
          '<button data-field="sustain_btn">Set</button>' +
        '</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Phase</span>' +
        '<span class="value" data-field="phase">\u2014</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Triggered</span>' +
        '<span class="value" data-field="triggered">\u2014</span>' +
      '</div>' +
      '<div class="row">' +
        '<span class="label">Process</span>' +
        '<span class="value" data-field="process_us">\u2014</span>' +
      '</div>' +
      '<div class="sparkline-wrap">' +
        '<canvas data-field="sparkline"></canvas>' +
      '</div>' +
      // Real-time stats section
      '<div class="stats-section">' +
        '<div class="stats-header">' +
          '<span>Real-time (60s)</span>' +
          '<button data-field="resetBtn" class="reset-btn">Reset Pipeline</button>' +
        '</div>' +
        '<div class="stats-grid">' +
          '<span class="label">Energy</span>' +
          '<span data-field="stats_energy">\u2014</span>' +
          '<span class="label">Floor</span>' +
          '<span data-field="stats_floor">\u2014</span>' +
          '<span class="label">Triggers</span>' +
          '<span data-field="stats_triggers">\u2014</span>' +
          '<span class="label">Avg Duration</span>' +
          '<span data-field="stats_duration">\u2014</span>' +
          '<span class="label">Last Trigger</span>' +
          '<span data-field="stats_last_trigger">\u2014</span>' +
        '</div>' +
      '</div>' +
      // 24h trend section
      '<div class="stats-section">' +
        '<div class="stats-header">' +
          '<span data-field="trend_header">Trend (24h) \u2014 waiting for data</span>' +
          '<a href="#" data-field="clearBtn" class="clear-link">clear</a>' +
        '</div>' +
        '<div class="stats-grid">' +
          '<span class="label">Floor Range</span>' +
          '<span data-field="trend_floor">\u2014</span>' +
          '<span class="label">Trigger Rate</span>' +
          '<span data-field="trend_triggers">\u2014</span>' +
          '<span class="label">Energy P95</span>' +
          '<span data-field="trend_energy">\u2014</span>' +
          '<span class="label">Sessions</span>' +
          '<span data-field="trend_sessions">\u2014</span>' +
        '</div>' +
      '</div>';

    this.card = card;

    // Cache element references
    const fields = card.querySelectorAll('[data-field]');
    for (const el of fields) {
      this.els[el.getAttribute('data-field')] = el;
    }

    // Wire up threshold controls
    const attackInput = this.els['attack_input'];
    const attackBtn = this.els['attack_btn'];
    const sustainInput = this.els['sustain_input'];
    const sustainBtn = this.els['sustain_btn'];

    attackBtn.addEventListener('click', () =>
      this._setRatio(attackInput, '/api/vad/attack_ratio'));
    attackInput.addEventListener('keydown', e => {
      if (e.key === 'Enter') this._setRatio(attackInput, '/api/vad/attack_ratio');
    });
    sustainBtn.addEventListener('click', () =>
      this._setRatio(sustainInput, '/api/vad/sustain_ratio'));
    sustainInput.addEventListener('keydown', e => {
      if (e.key === 'Enter') this._setRatio(sustainInput, '/api/vad/sustain_ratio');
    });

    // Wire up reset button
    this.els['resetBtn'].addEventListener('click', () => this._resetPipeline());

    // Wire up clear history link (hides trend until next fetch)
    this.els['clearBtn'].addEventListener('click', (e) => {
      e.preventDefault();
      this._serverRollups = [];
      this.els['trend_header'].textContent = 'Trend (24h) \u2014 waiting for data';
      this.els['trend_floor'].textContent = '\u2014';
      this.els['trend_triggers'].textContent = '\u2014';
      this.els['trend_energy'].textContent = '\u2014';
      this.els['trend_sessions'].textContent = '\u2014';
    });

    // Redraw sparkline on resize
    this._resizeHandler = () => this._scheduleSparkline();
    window.addEventListener('resize', this._resizeHandler);

    return card;
  }

  /** Clean up event listeners. */
  destroy() {
    if (this._resizeHandler) {
      window.removeEventListener('resize', this._resizeHandler);
    }
  }

  /**
   * Update the card with new tick data for this stage.
   * @param {object} stats
   */
  update(stats) {
    const energy = stats.energy;
    this.els['energy'].textContent = energy != null ? Math.round(energy) : '\u2014';

    if (stats.noise_floor != null) {
      this.els['noise_floor'].textContent = Math.round(stats.noise_floor);
    }

    if (stats.attack_threshold != null) {
      this.attackThreshold = stats.attack_threshold;
      this.els['attack_threshold'].textContent = Math.round(stats.attack_threshold);
    }

    if (stats.sustain_threshold != null) {
      this.sustainThreshold = stats.sustain_threshold;
      this.els['sustain_threshold'].textContent = Math.round(stats.sustain_threshold);
    }

    // Phase: 0.0 = attack, 1.0 = sustain
    const phaseEl = this.els['phase'];
    if (stats.phase != null) {
      if (stats.phase === 0 || stats.phase === 0.0) {
        phaseEl.innerHTML = '<span class="dot dot-red"></span>attack';
      } else {
        phaseEl.innerHTML = '<span class="dot dot-amber"></span>sustain';
      }
    } else {
      phaseEl.textContent = '\u2014';
    }

    // Triggered: 1.0 = yes, 0.0 = no
    const triggeredEl = this.els['triggered'];
    if (stats.triggered != null && stats.triggered > 0) {
      triggeredEl.innerHTML = '<span class="dot dot-amber"></span>Yes';
    } else {
      triggeredEl.textContent = 'No';
    }

    if (stats.process_us != null) {
      this.els['process_us'].textContent = Math.round(stats.process_us) + '\u00b5s';
    }

    // Energy + floor history for sparkline and stats
    this.energyHistory.push(energy != null ? energy : 0);
    if (this.energyHistory.length > MAX_ENERGY_SAMPLES) this.energyHistory.shift();

    this.noiseFloorHistory.push(stats.noise_floor != null ? stats.noise_floor : 0);
    if (this.noiseFloorHistory.length > MAX_ENERGY_SAMPLES) this.noiseFloorHistory.shift();

    // Track trigger events
    this.triggerTracker.push(stats.triggered != null && stats.triggered > 0);

    // Real-time stats update (~1Hz)
    this.statsCounter++;
    if (this.statsCounter >= STATS_UPDATE_INTERVAL) {
      this.statsCounter = 0;
      this._updateRealtimeStats();
    }

    this._scheduleSparkline();
  }

  /** Compute and display real-time stats from the 60s raw buffers. */
  _updateRealtimeStats() {
    const e = computeStats(this.energyHistory);
    this.els['stats_energy'].textContent =
      'min=' + Math.round(e.min) + ' p50=' + Math.round(e.p50) +
      ' p95=' + Math.round(e.p95) + ' max=' + Math.round(e.max);

    const f = computeStats(this.noiseFloorHistory);
    this.els['stats_floor'].textContent =
      Math.round(f.mean) + ' (range ' + Math.round(f.min) + '\u2013' + Math.round(f.max) + ')';

    const t = this.triggerTracker.stats;
    this.els['stats_triggers'].textContent =
      t.totalTriggers + ' events, ' + t.triggerPercent.toFixed(1) + '% rate';
    this.els['stats_duration'].textContent =
      fmtDuration(t.avgDurationMs) + ' (max ' + fmtDuration(t.maxDurationMs) + ')';
    this.els['stats_last_trigger'].textContent = fmtAgo(t.lastTriggerAgo);
  }

  /**
   * Accept server-side rollup entries and update the trend display.
   * Called on SSE connect and periodically (60s).
   * @param {Array} entries - RollupEntry objects from GET /api/rollups
   */
  setRollups(entries) {
    this._serverRollups = entries;
    this._updateTrendDisplay();
  }

  /** Update the 24h trend display from server rollup entries. */
  _updateTrendDisplay() {
    const entries = this._serverRollups;
    if (!entries || entries.length === 0) {
      this.els['trend_header'].textContent = 'Trend (24h) \u2014 waiting for data';
      return;
    }

    let floorMin = Infinity, floorMax = -Infinity;
    let energyP95Min = Infinity, energyP95Max = -Infinity;
    let peakTriggerPct = 0, peakUptimeSecs = 0;
    let triggerPctSum = 0;
    let totalSessions = 0, totalFalseSessions = 0;

    for (const e of entries) {
      if (e.floor_min < floorMin) floorMin = e.floor_min;
      if (e.floor_max > floorMax) floorMax = e.floor_max;
      if (e.energy_p95 < energyP95Min) energyP95Min = e.energy_p95;
      if (e.energy_p95 > energyP95Max) energyP95Max = e.energy_p95;
      const pct = e.total_frames > 0 ? (e.triggered_frames / e.total_frames) * 100 : 0;
      triggerPctSum += pct;
      if (pct > peakTriggerPct) {
        peakTriggerPct = pct;
        peakUptimeSecs = e.uptime_secs;
      }
      totalSessions += (e.session_count || 0);
      totalFalseSessions += (e.false_session_count || 0);
    }

    const minutes = entries.length;
    const avgPct = triggerPctSum / minutes;

    // Approximate wall-clock time for peak trigger entry
    const currentUptime = snapshotData?.uptime_seconds || 0;
    const peakDate = new Date(Date.now() - (currentUptime - peakUptimeSecs) * 1000);

    this.els['trend_header'].textContent =
      'Trend (24h) \u2014 ' + minutes + ' minute' + (minutes !== 1 ? 's' : '') + ' captured';
    this.els['trend_floor'].textContent =
      Math.round(floorMin) + '\u2013' + Math.round(floorMax);
    this.els['trend_triggers'].textContent =
      avgPct.toFixed(1) + '% avg, ' + peakTriggerPct.toFixed(1) + '% peak (' + fmtTime(peakDate) + ')';
    this.els['trend_energy'].textContent =
      Math.round(energyP95Min) + '\u2013' + Math.round(energyP95Max) + ' range';

    const falsePct = totalSessions > 0 ? (totalFalseSessions / totalSessions * 100).toFixed(0) : '0';
    this.els['trend_sessions'].textContent =
      totalSessions + ' total, ' + totalFalseSessions + ' false (' + falsePct + '%)';
  }

  /**
   * Return current stats for dump export.
   * @returns {{ realtime: object, trend: object, rollupCSV: string }}
   */
  getStats() {
    const entries = this._serverRollups || [];
    const trend = { minutes: entries.length };

    // Generate CSV from server rollup entries
    let rollupCSV = '';
    if (entries.length > 0) {
      const header = 'uptime_secs,floor_min,floor_max,floor_mean,energy_p95,trigger_count,trigger_pct';
      const rows = entries.map(e => {
        const pct = e.total_frames > 0 ? (e.triggered_frames / e.total_frames) * 100 : 0;
        return [e.uptime_secs, Math.round(e.floor_min), Math.round(e.floor_max),
          Math.round(e.floor_mean), Math.round(e.energy_p95),
          e.trigger_count, pct.toFixed(1)].join(',');
      });
      rollupCSV = [header, ...rows].join('\n');
    }

    return {
      realtime: {
        energy: computeStats(this.energyHistory),
        floor: computeStats(this.noiseFloorHistory),
        trigger: this.triggerTracker.stats,
      },
      trend,
      rollupCSV,
    };
  }

  _scheduleSparkline() {
    if (!this.sparklineDirty) {
      this.sparklineDirty = true;
      requestAnimationFrame(() => {
        this.sparklineDirty = false;
        const canvas = this.els['sparkline'];
        if (!canvas) return;
        const visible = this.energyHistory.slice(-SPARKLINE_VISIBLE);
        drawSparkline(canvas, visible, this.attackThreshold, this.sustainThreshold, SPARKLINE_VISIBLE);
      });
    }
  }

  async _setRatio(input, endpoint) {
    const val = parseFloat(input.value);
    if (!val || val <= 0) {
      flash(input, 'flash-err');
      return;
    }
    try {
      const res = await fetch(endpoint, { method: 'POST', body: String(val) });
      if (!res.ok) throw new Error(res.status);
      flash(input, 'flash-ok');
      input.value = '';
    } catch (err) {
      flash(input, 'flash-err');
    }
  }

  async _resetPipeline() {
    const btn = this.els['resetBtn'];
    try {
      const res = await fetch('/api/pipeline/reset', { method: 'POST' });
      if (!res.ok) throw new Error(res.status);
      flash(btn, 'flash-ok');
      this.triggerTracker.reset();
    } catch (err) {
      flash(btn, 'flash-err');
    }
  }

  _prettyName(name) {
    return name.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
  }

  _escapeHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
}
