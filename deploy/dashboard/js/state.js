export const MAX_ENERGY_SAMPLES = 3000;
export const SPARKLINE_VISIBLE = 500;
export const WORK_HISTORY_SIZE = 50;

export const stateLog = [];
export const workUsHistory = [];

export let lastState = null;
export function setLastState(v) { lastState = v; }

export let snapshotData = {};
export function setSnapshotData(v) { snapshotData = v; }

export let currentTick = {};
export function setCurrentTick(v) { currentTick = v; }

// Stage renderers keyed by stage name
export const stageRenderers = {};

export const $ = id => document.getElementById(id);

export function formatDuration(secs) {
  if (secs == null) return '\u2014';
  if (secs < 1) return '<1s';
  if (secs < 60) return Math.floor(secs) + 's';
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  if (m < 60) return m + 'm ' + s + 's';
  const h = Math.floor(m / 60);
  return h + 'h ' + (m % 60) + 'm';
}

export function formatAgo(secs) {
  if (secs == null) return '\u2014';
  return formatDuration(secs) + ' ago';
}

export function stateClass(s) {
  const lower = (s || '').toLowerCase();
  if (lower === 'idle') return 'idle';
  if (lower === 'streaming') return 'streaming';
  if (lower === 'processing') return 'processing';
  if (lower === 'responding') return 'responding';
  return 'idle';
}

export function timeStr() {
  return new Date().toLocaleTimeString([], {
    hour: '2-digit', minute: '2-digit', second: '2-digit'
  });
}
