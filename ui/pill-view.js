// Renders the pill for a state. Used by the real pill window and the
// live preview on the Indicator page, so both always match.

const CHECK =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';
const ALERT =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><path d="M12 6.5v7"/><path d="M12 17.5v.01"/></svg>';
const CLIP =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="4" width="10" height="16" rx="2"/><path d="M10 4h4"/></svg>';

export const WAVE_BARS = 9;

/** The purple Wirl logo (icons.json default). */
const DEFAULT_ICON = "assets/icons/swirl.png";

/** The model's short label in the badge's place (quick switch). */
function tagBadge(tag) {
  const t = String(tag ?? "");
  return `<div class="badge"><span class="tag${t.length > 2 ? " long" : ""}">${esc(t)}</span></div>`;
}

function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

/**
 * @param {HTMLElement} el   the .pill element
 * @param {object} state     PillState from Rust: {kind, ...}
 * @param {{icon?: string, size?: string}} opts
 */
export function renderPill(el, state, opts = {}) {
  const icon = opts.icon ? `assets/icons/${encodeURIComponent(opts.icon)}` : null;
  el.className = `pill size-${opts.size || "medium"}`;
  // The icon is always shown plain: the pill appearing and the moving wave
  // already say "listening".
  // Falls back to the purple Wirl logo if no icon is set or it can't load.
  const iconImg = () =>
    `<div class="badge"><img src="${icon || DEFAULT_ICON}" alt="" onerror="if(!this.dataset.f){this.dataset.f=1;this.src='${DEFAULT_ICON}'}"></div>`;
  let html = "";
  switch (state.kind) {
    case "recording": {
      const bars = Array.from({ length: WAVE_BARS }, () => "<span></span>").join("");
      // Icon and wave only, like the bubble on the phone. Right after a quick
      // switch the model's tag (EN, 粵, ...) stands in for the icon for a
      // moment, with the model's name beside the wave; then the icon is back.
      const badge = state.tag ? tagBadge(state.tag) : iconImg();
      const name = state.name ? `<div class="text"><span class="sub">${esc(state.name)}</span></div>` : "";
      html = `${badge}<div class="wave">${bars}</div>${name}`;
      break;
    }
    case "modelSwitched":
      html = `${tagBadge(state.tag)}<div class="text">${esc(state.name)}${state.note ? `<span class="sub">${esc(state.note)}</span>` : ""}</div>`;
      break;
    case "processing":
      html = `<div class="symbol"><div class="spinner"></div></div><div class="text">Cleaning up</div>`;
      break;
    case "inserted":
      html = `<div class="symbol ok">${CHECK}</div><div class="text">Inserted</div>`;
      break;
    case "undone":
      html = `<div class="symbol ok">${CHECK}</div><div class="text">Undone</div>`;
      break;
    case "insertedWithTip":
      html = `<div class="symbol ok">${CHECK}</div><div class="text">Inserted<span class="sub">${esc(state.tip)}</span></div>`;
      break;
    case "cleaned":
      html = `<div class="symbol ok">${CHECK}</div><div class="text">Cleaned<span class="sub">${esc(state.undoHint)}</span></div>`;
      break;
    case "copied":
      el.classList.add("warn");
      html = `<div class="symbol warn">${CLIP}</div><div class="text warn-text">Copied, press Ctrl+V</div>`;
      break;
    case "error":
      el.classList.add("warn");
      html = `<div class="symbol warn">${ALERT}</div><div class="text warn-text">${esc(state.message)}</div>`;
      break;
    default:
      html = "";
  }
  el.innerHTML = html;
}

/** Animates the wave bars from an RMS level (0..~0.5). */
export function makeWave() {
  const history = new Array(WAVE_BARS).fill(0);
  return function update(el, rms) {
    const level = Math.min(1, Math.sqrt(Math.max(0, rms)) * 2.4);
    history.shift();
    history.push(level);
    const bars = el.querySelectorAll(".wave span");
    bars.forEach((b, i) => {
      const center = 1 - Math.abs(i - (WAVE_BARS - 1) / 2) / WAVE_BARS;
      const h = 0.22 + history[i] * 1.35 * (0.6 + 0.4 * center);
      b.style.height = `${Math.min(1.6, h).toFixed(2)}em`;
    });
  };
}
