/**
 * Generic stage renderer for unknown pipeline stages.
 *
 * Displays all stats as key/value rows. Values ending in _us are
 * formatted as microseconds; all others are shown with up to 4
 * decimal places.
 */

export class GenericRenderer {
  constructor() {
    this.card = null;
    this.body = null;
    this.els = {};
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
      '<div data-role="body"></div>';

    this.card = card;
    this.body = card.querySelector('[data-role="body"]');
    return card;
  }

  /**
   * Update the card with new tick data for this stage.
   * @param {object} stats
   */
  update(stats) {
    for (const [key, value] of Object.entries(stats)) {
      let el = this.els[key];
      if (!el) {
        // Create row on first sight
        const row = document.createElement('div');
        row.className = 'row';
        const label = document.createElement('span');
        label.className = 'label';
        label.textContent = this._prettyLabel(key);
        const val = document.createElement('span');
        val.className = 'value';
        row.appendChild(label);
        row.appendChild(val);
        this.body.appendChild(row);
        this.els[key] = val;
        el = val;
      }
      el.textContent = this._formatValue(key, value);
    }
  }

  _formatValue(key, value) {
    if (typeof value !== 'number') return String(value);
    if (key.endsWith('_us') || key === 'process_us') {
      return Math.round(value) + '\u00b5s';
    }
    if (Number.isInteger(value)) return String(value);
    return value.toFixed(4);
  }

  _prettyName(name) {
    return name.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
  }

  _prettyLabel(key) {
    return key.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
  }

  _escapeHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
}
