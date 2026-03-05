/**
 * Spectrogram renderer for the MFCC pipeline stage.
 *
 * Displays a scrolling waterfall of 26 mel-band energies received as
 * base64-encoded u8 values via the "mel" SSE event (10Hz).
 * Also shows spectral centroid and flatness from regular tick stats.
 */

// Precompute a 256-entry inferno-ish colormap (R,G,B per index).
const COLORMAP = buildColormap();

function buildColormap() {
  const map = new Uint8Array(256 * 3);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    // Black → dark purple → red-orange → yellow → white
    let r, g, b;
    if (t < 0.25) {
      const s = t / 0.25;
      r = Math.floor(s * 80);
      g = 0;
      b = Math.floor(s * 120);
    } else if (t < 0.5) {
      const s = (t - 0.25) / 0.25;
      r = Math.floor(80 + s * 160);
      g = Math.floor(s * 40);
      b = Math.floor(120 - s * 80);
    } else if (t < 0.75) {
      const s = (t - 0.5) / 0.25;
      r = Math.floor(240 + s * 15);
      g = Math.floor(40 + s * 170);
      b = Math.floor(40 - s * 40);
    } else {
      const s = (t - 0.75) / 0.25;
      r = 255;
      g = Math.floor(210 + s * 45);
      b = Math.floor(s * 180);
    }
    map[i * 3] = Math.min(255, r);
    map[i * 3 + 1] = Math.min(255, g);
    map[i * 3 + 2] = Math.min(255, b);
  }
  return map;
}

export class SpectrogramRenderer {
  constructor() {
    this._canvas = null;
    this._ctx = null;
    this._card = null;
    this._centroidEl = null;
    this._flatnessEl = null;
    this._nBands = 26;
    this._ready = false;
  }

  create(name) {
    const card = document.createElement('div');
    card.className = 'stage-card';
    card.style.gridColumn = '1 / -1'; // span full width

    card.innerHTML = `
      <div class="card-title">${name} — spectrogram</div>
      <div class="row">
        <span class="label">Spectral Centroid</span>
        <span class="value" data-field="spectral_centroid">—</span>
      </div>
      <div class="row">
        <span class="label">Spectral Flatness</span>
        <span class="value" data-field="spectral_flatness">—</span>
      </div>
      <div class="sparkline-wrap">
        <canvas class="spectrogram-canvas"></canvas>
      </div>
      <div style="display:flex;justify-content:space-between;margin-top:0.2rem;">
        <span class="label" style="font-size:0.65rem">8000 Hz</span>
        <span class="label" style="font-size:0.65rem">← mel bands →</span>
        <span class="label" style="font-size:0.65rem">0 Hz</span>
      </div>
    `;

    this._card = card;
    this._centroidEl = card.querySelector('[data-field="spectral_centroid"]');
    this._flatnessEl = card.querySelector('[data-field="spectral_flatness"]');
    this._canvas = card.querySelector('.spectrogram-canvas');
    this._canvas.style.height = '80px';

    // Defer canvas setup until it's in the DOM and has dimensions
    requestAnimationFrame(() => this._initCanvas());

    return card;
  }

  _initCanvas() {
    const canvas = this._canvas;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.floor(rect.width * dpr);
    const h = Math.floor(rect.height * dpr);
    if (w === 0 || h === 0) {
      // Not yet laid out — retry
      requestAnimationFrame(() => this._initCanvas());
      return;
    }

    canvas.width = w;
    canvas.height = h;
    this._ctx = canvas.getContext('2d');
    this._ctx.fillStyle = '#000';
    this._ctx.fillRect(0, 0, w, h);
    this._ready = true;
  }

  /** Called on every tick SSE event with stage stats (centroid, flatness, etc.) */
  update(stats) {
    if (stats.spectral_centroid != null) {
      this._centroidEl.textContent = Math.round(stats.spectral_centroid) + ' Hz';
    }
    if (stats.spectral_flatness != null) {
      this._flatnessEl.textContent = stats.spectral_flatness.toFixed(3);
    }
  }

  /** Called from the "mel" SSE event with a base64 string of quantized mel energies. */
  pushMel(base64Str) {
    if (!this._ready) return;

    // Decode base64 → u8 array
    const raw = atob(base64Str);
    const n = Math.min(raw.length, this._nBands);
    const bands = new Uint8Array(n);
    for (let i = 0; i < n; i++) {
      bands[i] = raw.charCodeAt(i);
    }

    const ctx = this._ctx;
    const canvas = this._canvas;
    const w = canvas.width;
    const h = canvas.height;
    const bandH = h / n;

    // Scroll left by 1 column: copy canvas shifted left, then draw new column
    ctx.drawImage(canvas, -1, 0);

    // Draw new column at the right edge (high freq at top, low at bottom)
    const x = w - 1;
    for (let i = 0; i < n; i++) {
      const val = bands[n - 1 - i]; // reverse: low freq at bottom
      const ci = val * 3;
      ctx.fillStyle = `rgb(${COLORMAP[ci]},${COLORMAP[ci + 1]},${COLORMAP[ci + 2]})`;
      ctx.fillRect(x, Math.floor(i * bandH), 1, Math.ceil(bandH));
    }
  }

  destroy() {
    this._ready = false;
    this._ctx = null;
  }
}
