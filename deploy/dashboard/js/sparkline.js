/**
 * Sparkline canvas renderer for VAD energy history.
 *
 * Draws the energy waveform (blue line) with attack (red) and sustain (amber)
 * threshold lines. Caller manages the history buffer and thresholds.
 */

const SPARKLINE_PAD = 4;

/**
 * Draw the energy sparkline on the given canvas.
 *
 * @param {HTMLCanvasElement} canvas
 * @param {number[]} history - Energy samples (newest last)
 * @param {number|null} attackThreshold
 * @param {number|null} sustainThreshold
 * @param {number} maxSamples - Total visible width in samples
 */
export function drawSparkline(canvas, history, attackThreshold, sustainThreshold, maxSamples) {
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = rect.width * dpr;
  canvas.height = rect.height * dpr;
  const ctx = canvas.getContext('2d');
  ctx.scale(dpr, dpr);
  const w = rect.width;
  const h = rect.height;
  const pad = SPARKLINE_PAD;

  ctx.clearRect(0, 0, w, h);

  if (history.length < 2) return;

  // Y scale: max of both thresholds or highest energy, with padding
  const maxEnergy = Math.max(...history, attackThreshold || 0, sustainThreshold || 0);
  const yMax = Math.max(maxEnergy * 1.15, 100);
  const styles = getComputedStyle(document.body);
  const monoFont = '9px ' + styles.getPropertyValue('--mono');
  const redColor = styles.getPropertyValue('--red').trim();
  const amberColor = styles.getPropertyValue('--amber').trim();
  const blueColor = styles.getPropertyValue('--blue').trim();

  // Attack threshold line (red)
  if (attackThreshold != null) {
    const ty = h - pad - ((attackThreshold / yMax) * (h - pad * 2));
    ctx.strokeStyle = redColor + '44';
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(0, ty);
    ctx.lineTo(w, ty);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = redColor + '88';
    ctx.font = monoFont;
    ctx.fillText('attack', 3, ty - 3);
  }

  // Sustain threshold line (amber)
  if (sustainThreshold != null) {
    const ty = h - pad - ((sustainThreshold / yMax) * (h - pad * 2));
    ctx.strokeStyle = amberColor + '44';
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(0, ty);
    ctx.lineTo(w, ty);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = amberColor + '88';
    ctx.font = monoFont;
    ctx.fillText('sustain', 3, ty - 3);
  }

  // Energy line (blue)
  ctx.strokeStyle = blueColor;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  const step = w / (maxSamples - 1);
  const offset = maxSamples - history.length;
  for (let i = 0; i < history.length; i++) {
    const x = (offset + i) * step;
    const y = h - pad - ((history[i] / yMax) * (h - pad * 2));
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
}
