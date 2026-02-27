/**
 * Protocol event timeline renderer.
 *
 * Renders a chronological list of Wyoming protocol events with direction
 * indicators, timestamps, event types, and summary fields.
 */

const MAX_EVENTS = 200;

/**
 * Format a timestamp for display.
 * @param {Date} date
 * @returns {string}
 */
function formatTime(date) {
  return date.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    fractionalSecondDigits: 3,
  });
}

/**
 * Manage a protocol event log and render it to a container element.
 */
export class Timeline {
  /** @param {HTMLElement} container */
  constructor(container) {
    this.container = container;
    this.events = [];
  }

  /**
   * Add a protocol event.
   * @param {{ dir: string, type: string, summary: string }} data
   */
  push(data) {
    this.events.unshift({
      time: new Date(),
      dir: data.dir,
      type: data.type,
      summary: data.summary || '',
    });
    if (this.events.length > MAX_EVENTS) {
      this.events.pop();
    }
    this.render();
  }

  /** Clear all events. */
  clear() {
    this.events = [];
    this.render();
  }

  /** Render events to the container. */
  render() {
    if (this.events.length === 0) {
      this.container.innerHTML = '<li><span class="ts">no events yet</span></li>';
      return;
    }
    this.container.innerHTML = this.events.map(e => {
      const arrow = e.dir === 'out'
        ? '<span class="proto-out">\u2192</span>'
        : '<span class="proto-in">\u2190</span>';
      const summary = e.summary
        ? ' <span class="proto-summary">' + escapeHtml(e.summary) + '</span>'
        : '';
      return '<li>'
        + '<span class="ts">' + formatTime(e.time) + '</span>'
        + '<span>' + arrow + ' <strong>' + escapeHtml(e.type) + '</strong>' + summary + '</span>'
        + '</li>';
    }).join('');
  }
}

/**
 * @param {string} s
 * @returns {string}
 */
function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
