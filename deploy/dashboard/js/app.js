/**
 * Wyoming Satellite Dashboard — main application module.
 *
 * Connects to the satellite via SSE for real-time tick data and protocol events.
 * Falls back to 1s polling if SSE is unavailable.
 */

import { drawSparkline } from './sparkline.js';
import { Timeline } from './timeline.js';

// ── Configuration ──────────────────────────────────────────────────────────
const MAX_ENERGY_SAMPLES = 3000; // ~60s at 50Hz
const SPARKLINE_VISIBLE = 500;   // visible window in sparkline (~10s)
const POLL_FALLBACK_MS = 1000;

// ── State ──────────────────────────────────────────────────────────────────
let attackThreshold = null;
let sustainThreshold = null;
const energyHistory = [];
const stateLog = [];
const workUsHistory = [];  // rolling window for timing stats
const WORK_HISTORY_SIZE = 50;
let lastState = null;
let sparklineDirty = false;

// ── DOM refs ───────────────────────────────────────────────────────────────
const $ = id => document.getElementById(id);

// ── Timeline ───────────────────────────────────────────────────────────────
const timeline = new Timeline($('protocolEvents'));

// ── Helpers ────────────────────────────────────────────────────────────────
function formatDuration(secs) {
  if (secs == null) return '\u2014';
  if (secs < 1) return '<1s';
  if (secs < 60) return Math.floor(secs) + 's';
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

function formatAgo(secs) {
  if (secs == null) return '\u2014';
  return formatDuration(secs) + ' ago';
}

function stateClass(s) {
  const lower = (s || '').toLowerCase();
  if (lower === 'idle') return 'idle';
  if (lower === 'streaming') return 'streaming';
  if (lower === 'processing') return 'processing';
  if (lower === 'responding') return 'responding';
  return 'idle';
}

function timeStr() {
  return new Date().toLocaleTimeString([], {
    hour: '2-digit', minute: '2-digit', second: '2-digit'
  });
}

// ── Sparkline rendering (throttled via rAF) ────────────────────────────────
function scheduleSparkline() {
  if (!sparklineDirty) {
    sparklineDirty = true;
    requestAnimationFrame(() => {
      sparklineDirty = false;
      const visible = energyHistory.slice(-SPARKLINE_VISIBLE);
      drawSparkline($('sparkline'), visible, attackThreshold, sustainThreshold, SPARKLINE_VISIBLE);
    });
  }
}

window.addEventListener('resize', scheduleSparkline);

// ── State history rendering ────────────────────────────────────────────────
function renderHistory() {
  const ul = $('stateHistory');
  if (stateLog.length === 0) {
    ul.innerHTML = '<li><span class="ts">no transitions yet</span></li>';
    return;
  }
  ul.innerHTML = stateLog.map(e =>
    '<li><span class="ts">' + e.time + '</span><span>' +
    (e.from || '?') + ' \u2192 ' + e.to + '</span></li>'
  ).join('');
}

// ── SSE event handlers ─────────────────────────────────────────────────────
function handleSnapshot(data) {
  $('satName').textContent = data.satellite_name || 'Wyoming Satellite';
  if (data.area) $('area').textContent = data.area;
  $('serverAddr').textContent = data.server_address || '\u2014';
  $('audioDevice').textContent = data.audio_device || '\u2014';
  $('audioFormat').textContent = data.audio_format || '\u2014';
  $('vadMode').textContent = data.vad_mode || '\u2014';

  attackThreshold = data.attack_threshold;
  sustainThreshold = data.sustain_threshold;
  $('vadAttackThreshold').textContent = attackThreshold != null ? attackThreshold : '\u2014';
  $('vadSustainThreshold').textContent = sustainThreshold != null ? sustainThreshold : '\u2014';

  // Show threshold controls for energy VAD
  const isEnergy = data.vad_mode === 'energy';
  $('attackControl').style.display = isEnergy ? '' : 'none';
  $('sustainControl').style.display = isEnergy ? '' : 'none';
}

function handleTick(data) {
  // Energy history
  const energy = data.energy;
  energyHistory.push(energy != null ? energy : 0);
  if (energyHistory.length > MAX_ENERGY_SAMPLES) energyHistory.shift();

  // VAD display
  const phaseEl = $('vadPhase');
  if (data.phase === 'attack') {
    phaseEl.innerHTML = '<span class="dot dot-red"></span>attack';
  } else if (data.phase === 'sustain') {
    phaseEl.innerHTML = '<span class="dot dot-amber"></span>sustain';
  } else {
    phaseEl.textContent = '\u2014';
  }

  $('vadEnergy').textContent = energy != null ? energy : '\u2014';
  const triggered = $('vadTriggered');
  if (data.triggered) {
    triggered.innerHTML = '<span class="dot dot-amber"></span>Yes';
  } else {
    triggered.textContent = 'No';
  }

  // State
  const state = data.state;
  const badge = $('stateBadge');
  badge.textContent = state || '\u2014';
  badge.className = 'state-badge ' + stateClass(state);
  $('feedbackState').textContent = data.feedback || '\u2014';

  // Connection
  const dot = $('connDot');
  dot.className = 'dot ' + (data.connected ? 'dot-green' : 'dot-red');
  $('connStatus').textContent = data.connected ? 'Connected' : 'Disconnected';

  // State transitions
  if (state && state !== lastState) {
    stateLog.unshift({ time: timeStr(), from: lastState, to: state });
    if (stateLog.length > 30) stateLog.pop();
    renderHistory();
    lastState = state;
  }
  if (lastState === null) lastState = state;

  // Frame timing
  if (data.work_us != null) {
    workUsHistory.push(data.work_us);
    if (workUsHistory.length > WORK_HISTORY_SIZE) workUsHistory.shift();
    $('workCurrent').textContent = data.work_us + '\u00b5s';
    const avg = Math.round(workUsHistory.reduce((a, b) => a + b, 0) / workUsHistory.length);
    const max = Math.max(...workUsHistory);
    $('workAvg').textContent = avg + '\u00b5s';
    $('workMax').textContent = max + '\u00b5s';
    $('workHeadroom').textContent = ((20000 - avg) / 1000).toFixed(1) + 'ms / 20ms';
  }

  scheduleSparkline();
}

function handleProtocol(data) {
  timeline.push(data);
}

// ── SSE Connection ─────────────────────────────────────────────────────────
let pollInterval = null;

function connectSSE() {
  const es = new EventSource('/api/stream');

  es.addEventListener('snapshot', e => {
    $('banner').classList.remove('visible');
    handleSnapshot(JSON.parse(e.data));
  });

  es.addEventListener('tick', e => {
    handleTick(JSON.parse(e.data));
  });

  es.addEventListener('protocol', e => {
    handleProtocol(JSON.parse(e.data));
  });

  es.onerror = () => {
    $('banner').classList.add('visible');
    // EventSource will auto-reconnect. If it can't, fall back to polling.
    if (es.readyState === EventSource.CLOSED) {
      startPolling();
    }
  };
}

// ── Polling fallback ───────────────────────────────────────────────────────
function startPolling() {
  if (pollInterval) return;
  pollInterval = setInterval(async () => {
    try {
      const res = await fetch('/api/');
      if (!res.ok) throw new Error(res.status);
      const data = await res.json();
      $('banner').classList.remove('visible');

      // Map snapshot fields
      $('satName').textContent = data.satellite_name || 'Wyoming Satellite';
      const parts = [];
      if (data.area) parts.push(data.area);
      parts.push('up ' + formatDuration(data.uptime_seconds));
      $('area').textContent = parts.join(' \u00b7 ');

      // Connection
      const dot = $('connDot');
      dot.className = 'dot ' + (data.connected ? 'dot-green' : 'dot-red');
      $('connStatus').textContent = data.connected ? 'Connected' : 'Disconnected';
      $('serverAddr').textContent = data.server_address || '\u2014';
      $('lastPing').textContent = formatAgo(data.last_ping_secs);

      // State
      const badge = $('stateBadge');
      badge.textContent = data.state || '\u2014';
      badge.className = 'state-badge ' + stateClass(data.state);
      $('feedbackState').textContent = data.feedback_state || '\u2014';
      $('stateAge').textContent = formatDuration(data.last_state_change_secs);
      $('interactions').textContent = data.interaction_count != null ? data.interaction_count : '\u2014';

      // VAD
      $('vadMode').textContent = data.vad_mode || '\u2014';
      attackThreshold = data.vad_attack_threshold;
      sustainThreshold = data.vad_sustain_threshold;
      $('vadAttackThreshold').textContent = attackThreshold != null ? attackThreshold : '\u2014';
      $('vadSustainThreshold').textContent = sustainThreshold != null ? sustainThreshold : '\u2014';
      const isEnergy = data.vad_mode === 'energy';
      $('attackControl').style.display = isEnergy ? '' : 'none';
      $('sustainControl').style.display = isEnergy ? '' : 'none';

      const phaseEl = $('vadPhase');
      if (data.vad_phase === 'attack') {
        phaseEl.innerHTML = '<span class="dot dot-red"></span>attack';
      } else if (data.vad_phase === 'sustain') {
        phaseEl.innerHTML = '<span class="dot dot-amber"></span>sustain';
      } else {
        phaseEl.textContent = '\u2014';
      }

      const energy = data.vad_current_energy;
      $('vadEnergy').textContent = energy != null ? energy : '\u2014';
      const triggeredEl = $('vadTriggered');
      if (data.vad_triggered) {
        triggeredEl.innerHTML = '<span class="dot dot-amber"></span>Yes';
      } else {
        triggeredEl.textContent = 'No';
      }

      energyHistory.push(energy != null ? energy : 0);
      if (energyHistory.length > MAX_ENERGY_SAMPLES) energyHistory.shift();
      scheduleSparkline();

      // Audio
      $('audioDevice').textContent = data.audio_device || '\u2014';
      $('audioFormat').textContent = data.audio_format || '\u2014';

      // State history
      if (data.state && data.state !== lastState) {
        stateLog.unshift({ time: timeStr(), from: lastState, to: data.state });
        if (stateLog.length > 30) stateLog.pop();
        renderHistory();
        lastState = data.state;
      }
      if (lastState === null) lastState = data.state;
    } catch (err) {
      $('banner').classList.add('visible');
    }
  }, POLL_FALLBACK_MS);
}

// ── Threshold tuning ───────────────────────────────────────────────────────
async function setThresholdValue(inputId, endpoint) {
  const input = $(inputId);
  const val = parseInt(input.value, 10);
  if (!val || val < 1 || val > 65535) {
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

function flash(el, cls) {
  el.classList.remove('flash-ok', 'flash-err');
  void el.offsetWidth;
  el.classList.add(cls);
}

$('attackBtn').addEventListener('click', () =>
  setThresholdValue('attackInput', '/api/vad/attack_threshold'));
$('attackInput').addEventListener('keydown', e => {
  if (e.key === 'Enter') setThresholdValue('attackInput', '/api/vad/attack_threshold');
});
$('sustainBtn').addEventListener('click', () =>
  setThresholdValue('sustainInput', '/api/vad/sustain_threshold'));
$('sustainInput').addEventListener('keydown', e => {
  if (e.key === 'Enter') setThresholdValue('sustainInput', '/api/vad/sustain_threshold');
});

// ── Init ───────────────────────────────────────────────────────────────────
connectSSE();
