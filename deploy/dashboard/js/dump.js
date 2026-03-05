import {
  $, stateLog, workUsHistory,
  snapshotData, currentTick,
  stageRenderers,
} from './state.js';

export function initDump(timeline) {
  $('copyDump').addEventListener('click', async (e) => {
    e.preventDefault();
    const text = buildDump(timeline);
    try {
      await navigator.clipboard.writeText(text);
      $('copyDump').textContent = 'copied!';
    } catch {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      document.body.removeChild(ta);
      $('copyDump').textContent = 'copied!';
    }
    setTimeout(() => { $('copyDump').textContent = 'copy current'; }, 1500);
  });
}

function buildDump(timeline) {
  const t = currentTick;
  const s = snapshotData;
  const lines = [];

  lines.push('Wyoming Satellite Dump (' + new Date().toISOString() + ')');
  lines.push('state=' + (t.state || '?') + ' feedback=' + (t.feedback || '?') + ' connected=' + (t.connected ? 'yes' : 'no'));

  // Audio info
  lines.push('audio: ' + (s.audio_device || '?') + ' ' + (s.audio_format || '?'));

  // Pipeline stages from current tick
  if (t.stages) {
    for (const [name, stats] of Object.entries(t.stages)) {
      const parts = Object.entries(stats).map(([k, v]) => {
        if (typeof v === 'number') {
          return k + '=' + (k.endsWith('_us') ? Math.round(v) : v.toFixed(4));
        }
        return k + '=' + v;
      });
      lines.push('stage.' + name + ': ' + parts.join(' '));
    }
  }

  if (workUsHistory.length > 0) {
    const avg = Math.round(workUsHistory.reduce((a, b) => a + b, 0) / workUsHistory.length);
    const max = Math.max(...workUsHistory);
    const current = workUsHistory[workUsHistory.length - 1];
    lines.push('timing: current=' + current + 'us avg=' + avg + 'us max=' + max + 'us headroom=' + ((20000 - avg) / 1000).toFixed(1) + 'ms/20ms');
  }

  // Stats from renderers that support getStats()
  for (const [name, renderer] of Object.entries(stageRenderers)) {
    const stats = renderer.getStats?.();
    if (!stats) continue;

    lines.push('');
    lines.push('--- ' + name + ' real-time (60s) ---');

    if (stats.realtime) {
      const e = stats.realtime.energy;
      if (e && e.count > 0) {
        lines.push('energy: min=' + Math.round(e.min) + ' p50=' + Math.round(e.p50) +
          ' p95=' + Math.round(e.p95) + ' max=' + Math.round(e.max) +
          ' \u03c3=' + Math.round(e.stddev));
      }
      const f = stats.realtime.floor;
      if (f && f.count > 0) {
        lines.push('floor: ' + Math.round(f.mean) + ' (range ' + Math.round(f.min) + '-' + Math.round(f.max) + ')');
      }
      const tr = stats.realtime.trigger;
      if (tr) {
        lines.push('triggers: ' + tr.totalTriggers + ' events, avg ' +
          (tr.avgDurationMs / 1000).toFixed(1) + 's, max ' +
          (tr.maxDurationMs / 1000).toFixed(1) + 's, rate ' +
          tr.triggerPercent.toFixed(1) + '%');
      }
    }

    if (stats.rollupCSV && stats.trend && stats.trend.minutes > 0) {
      lines.push('');
      lines.push('--- ' + name + ' trend (24h, ' + stats.trend.minutes + ' minutes) ---');
      lines.push(stats.rollupCSV);
    }
  }

  if (stateLog.length > 0) {
    lines.push('');
    const n = Math.min(stateLog.length, 10);
    lines.push('state transitions (last ' + n + '):');
    stateLog.slice(0, n).forEach(e => {
      lines.push('  ' + e.time + ' ' + (e.from || '?') + ' -> ' + e.to);
    });
  }

  if (timeline.events.length > 0) {
    lines.push('');
    const n = Math.min(timeline.events.length, 20);
    lines.push('protocol events (last ' + n + '):');
    timeline.events.slice(0, n).forEach(e => {
      const arrow = e.dir === 'out' ? '->' : '<-';
      const summary = e.summary ? ' (' + e.summary + ')' : '';
      const ts = e.time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
      lines.push('  ' + ts + ' ' + arrow + ' ' + e.type + summary);
    });
  }

  return lines.join('\n');
}
