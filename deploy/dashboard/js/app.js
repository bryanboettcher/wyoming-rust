import { Timeline } from './timeline.js';
import {
  $, stateClass, timeStr, formatDuration,
  stateLog, workUsHistory, WORK_HISTORY_SIZE,
  lastState, setLastState,
  setSnapshotData, setCurrentTick,
  stageRenderers,
} from './state.js';
import { getRenderer } from './renderers/registry.js';
import { initDump } from './dump.js';

const POLL_FALLBACK_MS = 1000;
const timeline = new Timeline($('protocolEvents'));

function handleSnapshot(data) {
  setSnapshotData(data);
  $('satName').textContent = data.satellite_name || 'Wyoming Satellite';
  if (data.area) $('area').textContent = data.area;
  $('serverAddr').textContent = data.server_address || '\u2014';
  $('audioDevice').textContent = data.audio_device || '\u2014';
  $('audioFormat').textContent = data.audio_format || '\u2014';

  // Create dynamic stage cards
  const container = $('stageContainer');
  container.innerHTML = '';
  // Destroy and clear old renderers
  for (const key of Object.keys(stageRenderers)) {
    if (stageRenderers[key].destroy) stageRenderers[key].destroy();
    delete stageRenderers[key];
  }

  if (data.stages && data.stages.length > 0) {
    for (const stageName of data.stages) {
      const renderer = getRenderer(stageName);
      const card = renderer.create(stageName);
      container.appendChild(card);
      stageRenderers[stageName] = renderer;
    }
  }
}

function handleTick(data) {
  setCurrentTick(data);

  // State badge
  const state = data.state;
  const badge = $('stateBadge');
  badge.textContent = state || '\u2014';
  badge.className = 'state-badge ' + stateClass(state);
  $('feedbackState').textContent = data.feedback || '\u2014';

  // Connection
  const dot = $('connDot');
  dot.className = 'dot ' + (data.connected ? 'dot-green' : 'dot-red');
  $('connStatus').textContent = data.connected ? 'Connected' : 'Disconnected';

  // State history
  if (state && state !== lastState) {
    stateLog.unshift({ time: timeStr(), from: lastState, to: state });
    if (stateLog.length > 30) stateLog.pop();
    renderHistory();
    setLastState(state);
  }
  if (lastState === null) setLastState(state);

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

  // Update dynamic stage cards
  if (data.stages) {
    for (const [stageName, stats] of Object.entries(data.stages)) {
      const renderer = stageRenderers[stageName];
      if (renderer) renderer.update(stats);
    }
  }
}

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

function handleMel(base64Str) {
  const renderer = stageRenderers['mfcc'];
  if (renderer && renderer.pushMel) {
    renderer.pushMel(base64Str);
  }
}

function handleProtocol(data) {
  timeline.push(data);
}

let pollInterval = null;
let rollupInterval = null;

async function fetchRollups() {
  try {
    const res = await fetch('/api/rollups');
    if (!res.ok) return;
    const entries = await res.json();
    for (const renderer of Object.values(stageRenderers)) {
      if (renderer.setRollups) renderer.setRollups(entries);
    }
  } catch {}
}

function connectSSE() {
  const es = new EventSource('/api/stream');
  es.addEventListener('snapshot', e => {
    $('banner').classList.remove('visible');
    handleSnapshot(JSON.parse(e.data));
    fetchRollups();
    if (!rollupInterval) {
      rollupInterval = setInterval(fetchRollups, 60000);
    }
  });
  es.addEventListener('tick', e => {
    handleTick(JSON.parse(e.data));
  });
  es.addEventListener('mel', e => {
    handleMel(e.data);
  });
  es.addEventListener('protocol', e => {
    handleProtocol(JSON.parse(e.data));
  });
  es.onerror = () => {
    $('banner').classList.add('visible');
    if (es.readyState === EventSource.CLOSED) {
      startPolling();
    }
  };
}

function startPolling() {
  // Simple fallback: just poll snapshot + show basic info, no tick data
  if (pollInterval) return;
  pollInterval = setInterval(async () => {
    try {
      const res = await fetch('/api/');
      if (!res.ok) throw new Error(res.status);
      const data = await res.json();
      $('banner').classList.remove('visible');

      $('satName').textContent = data.satellite_name || 'Wyoming Satellite';
      const parts = [];
      if (data.area) parts.push(data.area);
      parts.push('up ' + formatDuration(data.uptime_seconds));
      $('area').textContent = parts.join(' \u00b7 ');

      const d = $('connDot');
      d.className = 'dot ' + (data.connected ? 'dot-green' : 'dot-red');
      $('connStatus').textContent = data.connected ? 'Connected' : 'Disconnected';
      $('serverAddr').textContent = data.server_address || '\u2014';

      const badge = $('stateBadge');
      badge.textContent = data.state || '\u2014';
      badge.className = 'state-badge ' + stateClass(data.state);
      $('feedbackState').textContent = data.feedback_state || '\u2014';
      $('audioDevice').textContent = data.audio_device || '\u2014';
      $('audioFormat').textContent = data.audio_format || '\u2014';

      if (data.state && data.state !== lastState) {
        stateLog.unshift({ time: timeStr(), from: lastState, to: data.state });
        if (stateLog.length > 30) stateLog.pop();
        renderHistory();
        setLastState(data.state);
      }
      if (lastState === null) setLastState(data.state);
    } catch (err) {
      $('banner').classList.add('visible');
    }
  }, POLL_FALLBACK_MS);
}

initDump(timeline);
connectSSE();
