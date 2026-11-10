import { renderPill, makeWave } from "./pill-view.js";
import {
  DICTATION_SCRIPT,
  MESSY_TEXT,
  tourSteps,
  dictationSummary,
  cleanState,
  dictateInstruction,
  pillWhere,
} from "./tour-logic.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const main = document.getElementById("main");
const PAGES = ["home", "dictation", "cleanup", "dictionary", "hotkeys", "indicator", "setup", "about", "tour"];

let S = null;
let page = PAGES.includes(location.hash.slice(1)) ? location.hash.slice(1) : "home";
let renderPending = false;
let previewTimer = null;
const ui = {
  search: "",
  adding: false,
  editing: null,
  dictError: "",
  pairing: null,
  capture: null,
  keyMsg: {},
  devClicks: 0,
  apiEditing: false,
  apiError: "",
  apiBusy: false,
  confirmReset: false,
  testText: "",
  tour: null,
  confirmDisconnect: false,
  /** Model whose Delete is waiting for a second click. */
  confirmDeleteModel: null,
};

// ---------- helpers ----------

const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function toast(msg) {
  const t = document.getElementById("toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), 2600);
}

function ago(ms) {
  if (!ms) return "never";
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.round(h / 24)} days ago`;
}

const POSITIONS = [
  ["bottomCenter", "Bottom center"],
  ["topCenter", "Top center"],
  ["nearMouse", "Near mouse"],
];
const SIZES = ["small", "medium", "large"];
const posLabel = (p) => (POSITIONS.find((x) => x[0] === p) || POSITIONS[0])[1];

function switchHtml(key, on) {
  return `<label class="switch"><input type="checkbox" data-setting="${key}" ${on ? "checked" : ""}><span></span></label>`;
}

function iconFile(id) {
  const set = S?.icons;
  if (!set) return null;
  const chosen = id || S.settings.icon || set.default;
  const e = set.icons.find((i) => i.id === chosen) || set.icons.find((i) => i.id === set.default) || set.icons[0];
  return e ? e.file : null;
}

async function save(patch) {
  try {
    await invoke("update_settings", { patch });
  } catch (e) {
    toast(String(e));
  }
}

function focusInMain() {
  const a = document.activeElement;
  return a && main.contains(a) && (a.tagName === "INPUT" || a.tagName === "TEXTAREA") && a.type !== "checkbox" && a.type !== "range";
}

async function load() {
  S = await invoke("get_state");
  render();
  patchLive();
}

// Bits that must update even while a text box has focus.
function patchLive() {
  const t = main.querySelector('[data-act="test"]');
  if (t) t.textContent = S.phase === "recording" ? "Stop" : "Test dictation";
  const ts = document.getElementById("tour-status");
  if (ts) ts.innerHTML = tourStatus();
  document.getElementById("setup-dot").hidden = attentionCount() === 0;
}

function attentionCount() {
  return S.attention.length;
}

// ---------- pages ----------

function pageHome() {
  const s = S.settings;
  const n = attentionCount();
  const phone = S.phones[0];
  const dictSummary = `${S.dictionary.count} term${S.dictionary.count === 1 ? "" : "s"}` + (phone ? ` · Synced with phone ${ago(phone.lastSync)}` : "");
  const cleanup = !s.cleanupEnabled ? "Off" : S.apiKey.saved ? "On" : "On · needs your API key";
  const cards = [
    ["dictation", "Dictation", S.modelSummary],
    ["cleanup", "AI cleanup", cleanup],
    ["dictionary", "Dictionary", dictSummary],
    ["hotkeys", "Hotkeys", `${S.hotkeys.dictate} to dictate`],
    ["indicator", "Indicator", `${posLabel(s.pillPosition)} · Chime ${s.chime ? "on" : "off"}`],
    ["setup", "Setup", n === 0 ? "All set" : `${n} thing${n === 1 ? "" : "s"} need${n === 1 ? "s" : ""} attention`],
  ];
  return `
  <div class="page">
    <div class="card status-card">
      <div class="row">
        <div class="grow">
          <div class="status-title">Wirl is ${s.enabled ? "on" : "off"}</div>
          <div class="muted" style="margin-top:4px">Press <kbd>${esc(S.hotkeys.dictate)}</kbd> in any text box to dictate</div>
        </div>
        ${switchHtml("enabled", s.enabled)}
      </div>
      <div class="divider"></div>
      ${
        n === 0
          ? `<div class="ok-text">Everything is set up</div>`
          : `<a class="warn-line link" data-go="setup" style="color:var(--warning)">${n} thing${n === 1 ? "" : "s"} need${n === 1 ? "s" : ""} attention</a>`
      }
    </div>
    ${showShortcutCard() ? shortcutCard() : ""}
    <div class="grid2">
      ${cards
        .map(
          ([p, title, sum]) => `<div class="card clickable" data-go="${p}">
            <h3>${title}</h3><div class="muted">${esc(sum)}</div></div>`,
        )
        .join("")}
    </div>
    <div class="row" style="margin-top:22px">
      <label class="check grow"><input type="checkbox" data-setting="startWithWindows" ${s.startWithWindows ? "checked" : ""}> Start with Windows</label>
      <span class="muted small">Version ${esc(S.version)}</span>
    </div>
  </div>`;
}

function showShortcutCard() {
  const u = S.settings.usage || {};
  return (u.dictations || 0) < 3 || (u.cleanups || 0) < 3;
}

function shortcutRows() {
  return `<div class="shortcut-row"><kbd>${esc(S.hotkeys.dictate)}</kbd><div><b>Talk, and it types</b><div class="muted small">${
    S.settings.holdToTalk ? "Hold while you speak, let go to insert." : "Press to start, press again to insert."
  }</div></div></div>
    <div class="shortcut-row"><kbd>${esc(S.hotkeys.cleanSelection)}</kbd><div><b>Tidy up selected text</b><div class="muted small">Press again right away to undo.</div></div></div>`;
}

function shortcutCard() {
  return `<div class="card">
    <div class="row"><h3 class="grow">Your two shortcuts</h3><a class="link small" data-act="tour-start">Take the tour again</a></div>
    <div class="stack" style="margin-top:10px">${shortcutRows()}</div>
  </div>`;
}

function modelCard(m) {
  const d = m.download || {};
  const pct = d.total ? Math.round((d.done / d.total) * 100) : 0;
  const partialPct = m.partial && m.total ? Math.round((m.partial / m.total) * 100) : 0;
  const confirming = ui.confirmDeleteModel === m.id;
  // Quick switch tick box: only a downloaded model can be ticked; the model
  // in use always counts as ticked.
  const tick = `<button class="tick ${m.ticked ? "on" : ""}" data-act="tick" data-id="${m.id}" ${m.ready ? "" : "disabled"}
      role="checkbox" aria-checked="${m.ticked}" aria-label="Quick switch: ${esc(m.name)}" title="${m.ready ? "Quick switch" : "Download it first"}">${m.ticked ? "✓" : ""}</button>`;
  let right;
  if (m.ready)
    right = `${m.inUse ? "" : `<button data-act="use-model" data-id="${m.id}">Use now</button>`}<button data-act="delete-model" data-id="${m.id}">Delete</button>`;
  else if (d.unpacking) right = "";
  else if (d.active) right = `<button data-act="cancel-download" data-id="${m.id}">Cancel</button>`;
  else if (m.partial) right = `<button data-act="download" data-id="${m.id}">Resume download</button>`;
  else right = `<button data-act="download" data-id="${m.id}">Download (${esc(m.sizeLabel)})</button>`;
  let status = "";
  if (m.ready && m.inUse) status = `<span class="ok-text">Ready · In use now</span>`;
  else if (m.ready && !m.ticked) status = "Ready · Tick to add it to quick switch";
  else if (m.ready) status = "Ready";
  // In use but not downloaded yet (or the download stopped): say so plainly.
  const needs =
    m.inUse && !m.ready && !d.active
      ? `<div class="warning-box" style="margin-top:12px">Not downloaded yet, so dictation won't work with this model. Click ${m.partial ? "Resume download" : `Download (${esc(m.sizeLabel)})`} to use it.</div>`
      : m.inUse && !m.ready
        ? `<div class="muted small" style="margin-top:6px">Dictation works as soon as the download finishes.</div>`
        : "";
  const deleteText = m.shared
    ? "Delete the Cantonese and Mandarin model? Both use the same files. You can download it again later."
    : `Delete the ${esc(m.name)} model? You can download it again later.`;
  const label = d.unpacking ? "Unpacking…" : `${pct}%`;
  return `<div class="card model-card ${m.inUse ? "selected" : ""}">
    <div class="row">
      ${tick}
      <div class="grow"><h3>${esc(m.name)}</h3>
        <div class="muted">${esc(m.description)}</div>
        <div class="muted small" style="margin-top:4px">${status ? `${status} · ` : ""}${esc(m.sizeNote)}</div></div>
      <div class="row" style="gap:8px;flex:none">${right}</div>
    </div>
    ${d.active ? `<div class="progress"><div data-progress="${esc(m.file)}" style="width:${d.unpacking ? 100 : pct}%"></div></div><div class="muted small" data-progress-label="${esc(m.file)}" style="margin-top:6px">${label}</div>` : ""}
    ${!d.active && !m.ready && m.partial ? `<div class="muted small" style="margin-top:6px">Partly downloaded (${partialPct}%). Resume to finish.</div>` : ""}
    ${d.error ? `<div class="error-text">${esc(d.error)}</div>` : ""}
    ${
      confirming
        ? `<div class="warning-box row" style="margin-top:12px"><div class="grow">${deleteText}</div>
            <button class="warn" data-act="delete-model-yes" data-id="${m.id}">Delete</button>
            <button data-act="delete-model-no" data-id="${m.id}">Keep</button></div>`
        : ""
    }
    ${needs}
  </div>`;
}

function pageDictation() {
  const s = S.settings;
  return `<div class="page">
    <h1>Dictation</h1>
    <p class="lead">Speech is turned into text on this computer. Pick the model for the language you speak; it downloads once.</p>
    <h2>Speech model</h2>
    <p class="muted" style="margin:-4px 0 12px">Tick the models you want to use. With more than one ticked, press <kbd>${esc(S.hotkeys.switchModel)}</kbd> to switch between them, even while recording.</p>
    ${S.models.map(modelCard).join("")}
    <div class="card row">
      <div class="grow"><h3>Pause media while dictating</h3><div class="muted">Music and videos pause while you talk and resume after.</div></div>
      ${switchHtml("pauseMedia", s.pauseMedia)}
    </div>
  </div>`;
}

function pageCleanup() {
  const s = S.settings;
  return `<div class="page">
    <h1>AI cleanup</h1>
    <p class="lead">Claude tidies what you said before it's inserted.</p>
    <div class="card row">
      <div class="grow"><h3>Clean up with Claude</h3><div class="muted">${s.cleanupEnabled ? "On" : "Off: your words are inserted exactly as heard."}</div></div>
      ${switchHtml("cleanupEnabled", s.cleanupEnabled)}
    </div>
    <div class="card">
      <h3>What it does</h3>
      <ul class="muted" style="line-height:1.7;margin:8px 0 0;padding-left:18px">
        <li>Keeps only what you meant when you correct yourself ("at 5, no actually 6" becomes "at 6").</li>
        <li>Removes false starts, repeats and filler words like "um" and "you know".</li>
        <li>Follows spoken commands such as "new paragraph" or "scratch that".</li>
        <li>Fixes words speech recognition got wrong, punctuation and capitals.</li>
        <li>Spells names and terms from your dictionary correctly.</li>
        <li>Keeps your tone. It never adds ideas of its own.</li>
      </ul>
      <div class="muted small" style="margin-top:10px">Only the text is sent to Anthropic, using your own API key. Audio never leaves this computer.</div>
    </div>
    <div class="card">
      <h3>Personal style notes</h3>
      <div class="muted" style="margin-bottom:10px">Optional. For example: "Use British spelling. I write casually to friends."</div>
      <textarea id="style-notes" placeholder="Your style notes">${esc(s.styleNotes)}</textarea>
    </div>
  </div>`;
}

function dictForm(e) {
  return `<div class="card stack" id="dict-form">
    <h3>${e ? "Edit term" : "Add term"}</h3>
    <label class="field"><span>Correct spelling</span>
      <input type="text" id="f-term" placeholder="What you want typed, e.g. Sela" value="${esc(e?.term)}"></label>
    <label class="field"><span>Wirl types it as</span>
      <input type="text" id="f-sounds" placeholder="The wrong words Wirl types instead, e.g. salad, Stella" value="${esc((e?.soundsLike || []).join(", "))}">
      <small>Copy them from what Wirl actually typed. Separate with commas.</small></label>
    <label class="field"><span>Only when <span class="muted">(optional)</span></span>
      <input type="text" id="f-note" placeholder="e.g. my brother's name, not the verb" value="${esc(e?.note)}">
      <small>Only needed if a wrong word is also a real word you sometimes mean.</small></label>
    ${ui.dictError ? `<div class="error-text">${esc(ui.dictError)}</div>` : ""}
    <div class="row"><button class="primary" data-act="dict-save">${e ? "Save" : "Add"}</button><button class="ghost" data-act="dict-cancel">Cancel</button></div>
  </div>`;
}

function pageDictionary() {
  const q = ui.search.trim().toLowerCase();
  const list = S.dictionary.entries.filter(
    (e) => !q || e.term.toLowerCase().includes(q) || e.note.toLowerCase().includes(q) || e.soundsLike.some((x) => x.toLowerCase().includes(q)),
  );
  const editing = ui.editing && S.dictionary.entries.find((e) => e.id === ui.editing);
  return `<div class="page">
    <h1>Dictionary</h1>
    <p class="lead">Names and words Wirl should always spell your way.</p>
    <div class="card sync-card">${syncCard()}</div>
    <div class="row" style="margin-bottom:14px">
      <input type="search" id="dict-search" class="grow" placeholder="Search ${S.dictionary.count} terms" value="${esc(ui.search)}">
      <button class="primary" data-act="dict-add">Add term</button>
      <button data-act="dict-import">Import</button>
      <button data-act="dict-export">Export</button>
    </div>
    ${ui.adding ? dictForm(null) : ""}
    ${editing ? dictForm(editing) : ""}
    <div class="card">
      ${
        list.length === 0
          ? `<div class="muted">${S.dictionary.count === 0 ? "No terms yet. Add names, places and words you use." : "No matches."}</div>`
          : list
              .map(
                (e) => `<div class="dict-row">
          <div class="grow"><div class="term">${esc(e.term)}</div>
            ${e.soundsLike.length ? `<div class="chips"><span class="muted small">typed as</span>${e.soundsLike.map((x) => `<span class="chip">${esc(x)}</span>`).join("")}</div>` : ""}
            ${e.note ? `<div class="muted small" style="margin-top:6px">${esc(e.note)}</div>` : ""}</div>
          <button class="ghost" data-act="dict-edit" data-id="${e.id}">Edit</button>
          <button class="ghost" data-act="dict-delete" data-id="${e.id}">Delete</button>
        </div>`,
              )
              .join("")
      }
    </div>
  </div>`;
}

/**
 * The Phone sync card, at the top of the Dictionary page: one compact row
 * (phones and status, Sync now, Pair), the pairing code when open.
 */
function syncCard() {
  const sy = S.sync || {};
  const phones = S.phones || [];
  let line;
  if (sy.syncing) line = `<span class="spinner-inline"></span><span class="muted">Syncing…</span>`;
  else if (sy.error) line = `<span style="color:var(--warning)">${esc(sy.error)}</span>`;
  else if (phones.length)
    line = `<span class="muted">${phones.map((p) => `${esc(p.name)} · synced ${ago(p.lastSync)}`).join(", ")}</span>`;
  else if (sy.active) line = `<span class="muted">Waiting for a phone to scan the code.</span>`;
  else line = `<span class="muted">Keep the same dictionary on your phone. Works anywhere, no account needed.</span>`;
  const disconnect = phones.length
    ? ui.confirmDisconnect
      ? `<a class="link small" style="color:var(--warning)" data-act="disconnect-all">Click again to disconnect all phones</a>`
      : `<a class="link small muted" data-act="disconnect-all">Disconnect all phones</a>`
    : "";
  const qr = ui.pairing
    ? `<div class="divider"></div><div class="row" style="align-items:flex-start;gap:22px">
        <div class="qr">${ui.pairing.svg}</div>
        <div class="grow stack">
          <div>On your phone: open Wirl › <b>Computer sync</b> › <b>Pair with a computer</b>, then scan this code.</div>
          <div class="muted small">Your phone doesn't need to be on the same Wi-Fi. Everything is encrypted on your devices before it's sent.</div>
          <div class="row" style="gap:16px"><button class="ghost" data-act="pair-close">Close</button>${disconnect}</div>
        </div></div>`
    : "";
  return `<div class="row" style="gap:12px">
      <div class="grow" style="min-width:0">
        <h3>Phone sync</h3>
        <div class="sync-line">${line}</div>
      </div>
      ${sy.active ? `<button class="ghost" data-act="sync-now" ${sy.syncing ? "disabled" : ""}>Sync now</button>` : ""}
      ${ui.pairing ? "" : `<button class="${phones.length ? "" : "primary"}" data-act="pair">${phones.length ? "Pair another" : "Pair a phone"}</button>`}
    </div>
    ${qr}`;
}

const HOTKEY_ROWS = [
  ["dictate", "Dictate", "Tap to start and stop. With Hold to talk on, hold it while you speak."],
  ["cancel", "Cancel", "Only while recording: throws the recording away."],
  ["cleanSelection", "Clean up selection", "Cleans the selected text. Press again right away to undo."],
  ["switchModel", "Switch speech model", "Moves to the next model ticked under Dictation. While recording, starts again in that model."],
];

function pageHotkeys() {
  const s = S.settings;
  return `<div class="page">
    <h1>Hotkeys</h1>
    <p class="lead">Click a shortcut, then press the new keys.</p>
    <div class="card">
      ${HOTKEY_ROWS.map(([key, label, desc], i) => {
        const listening = ui.capture === key;
        const msg = ui.keyMsg[key];
        const conflict = msg?.warning || S.hotkeys.conflicts[key];
        return `${i ? '<div class="divider"></div>' : ""}<div class="row">
          <div class="grow"><h3>${label}</h3><div class="muted">${desc}</div></div>
          <button class="shortcut ${listening ? "listening" : ""}" data-act="capture" data-id="${key}">${listening ? "Press new keys…" : esc(S.hotkeys[key])}</button>
        </div>
        ${msg?.error ? `<div class="error-text">${esc(msg.error)}</div>` : ""}
        ${conflict ? `<div class="warning-box">${esc(conflict)}. Choose another shortcut if it doesn't work.</div>` : ""}`;
      }).join("")}
    </div>
    <div class="card row">
      <div class="grow"><h3>Hold to talk</h3><div class="muted">Hold the Dictate shortcut to record, let go to insert. A quick tap still toggles.</div></div>
      ${switchHtml("holdToTalk", s.holdToTalk)}
    </div>
    <a class="link" data-act="hotkeys-reset">Reset to defaults</a>
  </div>`;
}

function pageIndicator() {
  const s = S.settings;
  const sizeIdx = Math.max(0, SIZES.indexOf(s.pillSize));
  const icons = S.icons?.icons || [];
  return `<div class="page">
    <h1>Indicator</h1>
    <p class="lead">The small pill that shows what Wirl is doing. It's hidden when Wirl is idle.</p>
    <div class="card"><div class="preview-stage"><div id="preview" class="pill"></div></div></div>
    <div class="card">
      <h3>Position</h3>
      <div class="segmented" style="margin-top:10px">
        ${POSITIONS.map(([v, l]) => `<button data-act="position" data-id="${v}" class="${s.pillPosition === v ? "on" : ""}">${l}</button>`).join("")}
      </div>
      <h3 style="margin-top:20px">Size</h3>
      <input type="range" min="0" max="2" step="1" value="${sizeIdx}" id="size-range">
      <div class="range-labels"><span>Small</span><span>Medium</span><span>Large</span></div>
    </div>
    <div class="card">
      <h3>Icon</h3>
      <div class="muted" style="margin-bottom:12px">Shown in the pill and the system tray.</div>
      ${
        icons.length
          ? `<div class="icon-grid">${icons
              .map((i) => `<button title="${esc(i.name)}" data-act="icon" data-id="${esc(i.id)}" class="${iconFile() === i.file ? "on" : ""}"><img src="assets/icons/${encodeURIComponent(i.file)}" alt="${esc(i.name)}"></button>`)
              .join("")}</div>`
          : `<div class="muted">Icons are missing from this build.</div>`
      }
    </div>
    <div class="card row">
      <div class="grow"><h3>Start and stop chime</h3><div class="muted">A soft sound when recording starts and stops.</div></div>
      ${switchHtml("chime", s.chime)}
    </div>
    <div class="card row">
      <div class="grow"><h3>Status messages</h3><div class="muted">Show "Inserted" and "Cleaned" after each action. Problems always show.</div></div>
      ${switchHtml("statusMessages", s.statusMessages)}
    </div>
  </div>`;
}

function checkItem(ok, title, body, fix) {
  return `<div class="check-item">
    <div class="check-icon ${ok ? "ok" : "warn"}">${ok ? "✓" : "!"}</div>
    <div class="grow"><h3>${title}</h3><div class="muted">${body}</div></div>
    ${ok ? "" : fix || ""}
  </div>`;
}

function apiKeyBody() {
  return S.apiKey.saved && !ui.apiEditing
      ? `${esc(S.apiKey.hint)} · <a class="link" data-act="api-change">Change</a> · <a class="link" data-act="api-remove">Remove</a>`
      : `Each person uses their own Anthropic API key. It's stored in Windows Credential Manager.
         <div class="row" style="margin-top:10px"><input type="password" id="api-key" class="grow" placeholder="sk-ant-..." autocomplete="off">
         <button class="primary" data-act="api-save" ${ui.apiBusy ? "disabled" : ""}>${ui.apiBusy ? "Checking…" : "Save"}</button></div>
         ${ui.apiError ? `<div class="error-text">${esc(ui.apiError)}</div>` : ""}
         <div class="small" style="margin-top:8px"><a class="link" data-act="open" data-url="https://console.anthropic.com/settings/keys">Get a key from the Anthropic Console</a></div>`;
}

function micItem() {
  const st = S.setup;
  return checkItem(
    st.microphone,
    "Microphone access",
    st.microphone ? "Wirl can hear your microphone." : "Wirl can't use a microphone. Plug one in, or allow desktop apps to use it in Windows privacy settings.",
    `<button class="warn" data-act="open" data-url="ms-settings:privacy-microphone">Fix</button>`,
  );
}

function pageSetup() {
  const st = S.setup;
  const model = S.models.find((m) => m.id === S.settings.model);
  const d = model?.download || {};
  const pct = d.total ? Math.round((d.done / d.total) * 100) : 0;
  const recording = S.phase === "recording";
  return `<div class="page">
    <h1>Setup</h1>
    <p class="lead">A few things Wirl needs. Green means done.</p>
    <div class="card row tour-invite">
      <div class="grow"><h3>New to Wirl?</h3><div class="muted">A 2-minute tour of talking and tidying up, with practice boxes.</div></div>
      <button class="primary" data-act="tour-start">Take the tour</button>
    </div>
    <div class="card">
      ${checkItem(st.apiKey, "API key", apiKeyBody(), "")}
      ${micItem()}
      ${checkItem(
        st.model,
        "Speech model",
        st.model
          ? `${esc(model.name)} model is ready.`
          : d.active
            ? `Downloading ${esc(model.name)}… ${pct}%<div class="progress"><div data-progress="${esc(model.file)}" style="width:${pct}%"></div></div>`
            : `The ${esc(model.name)} model (${esc(model.sizeLabel)}) needs to download once.${d.error ? `<div class="error-text">${esc(d.error)}</div>` : ""}`,
        d.active ? "" : `<button class="warn" data-act="download" data-id="${model.id}">Fix</button>`,
      )}
      ${checkItem(
        st.paired,
        "Phone pairing <span class='muted small'>(optional)</span>",
        st.paired
          ? `Paired with ${esc(S.phones.map((p) => p.name).join(", "))}.`
          : "Keep the same dictionary on Wirl for Android. Works anywhere with internet, no account needed.",
        `<button class="warn" data-act="pair-from-setup">Pair</button>`,
      )}
    </div>
    <div class="card stack">
      <h3>Test dictation</h3>
      <div class="muted">Click the box, press Test, say something, then press Stop.</div>
      <textarea id="test-box" placeholder="Your words appear here">${esc(ui.testText)}</textarea>
      <div><button class="primary" data-act="test">${recording ? "Stop" : "Test dictation"}</button></div>
    </div>
  </div>`;
}

function pageAbout() {
  const u = S.update;
  const dev = S.settings.developerOptions;
  const diag = S.diagnostics;
  return `<div class="page">
    <h1>About</h1>
    <div class="card">
      <div class="row">
        <div class="grow"><h3>Wirl for Windows</h3><div class="muted version" data-act="version">Version ${esc(S.version)}</div></div>
        ${
          u.readyVersion
            ? `<button class="primary" data-act="restart-update">Restart to update</button>`
            : `<button data-act="check-updates" ${u.checking ? "disabled" : ""}>${u.checking ? "Checking…" : "Check for updates"}</button>`
        }
      </div>
      ${u.message ? `<div class="muted small" style="margin-top:10px">${esc(u.message)}</div>` : ""}
    </div>
    <div class="card row">
      <div class="grow"><h3>Reset to defaults</h3><div class="muted">Resets all settings. Keeps your API key, dictionary and paired phones.</div></div>
      <button class="${ui.confirmReset ? "warn" : ""}" data-act="reset">${ui.confirmReset ? "Click again to reset" : "Reset"}</button>
    </div>
    ${
      dev && diag
        ? `<h2>Developer options</h2>
      <div class="card">
        <h3>Diagnostics</h3>
        <div class="muted small" style="margin:4px 0 12px">Speech engine: ${diag.sidecar.running ? "running" : "stopped"}${diag.sidecar.gpu ? " (GPU)" : " (CPU)"} · restarts: ${diag.sidecar.restarts}${diag.sidecar.lastError ? ` · last: ${esc(diag.sidecar.lastError)}` : ""}<br>Data folder: ${esc(diag.dataFolder)}</div>
        <table class="diag"><tr><th>When</th><th>What</th><th>Timing</th><th>Details</th></tr>
        ${diag.entries
          .map(
            (e) => `<tr><td>${new Date(e.atMs).toLocaleTimeString()}</td><td>${esc(e.summary)}</td>
          <td>${e.stages.map((s) => `${esc(s.name)}: ${s.ms} ms`).join("<br>")}</td>
          <td>${[e.model && `Model: ${esc(e.model)}`, e.thinking !== null && e.thinking !== undefined ? `Thinking: ${e.thinking ? "on" : "off"}` : "", e.insertMethod && `Insert: ${esc(e.insertMethod)}`, e.error && `<span class="warn-text" style="color:var(--warning)">${esc(e.error)}</span>`]
            .filter(Boolean)
            .join("<br>")}</td></tr>`,
          )
          .join("")}
        </table>
      </div>`
        : ""
    }
  </div>`;
}

// ---------- welcome tour ----------

const MIC_SVG =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="3" width="6" height="12" rx="3"/><path d="M5 11a7 7 0 0 0 14 0"/><path d="M12 18v3"/></svg>';
const SPARK_SVG =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/></svg>';

function initTour() {
  ui.tour = {
    steps: tourSteps(S.setup),
    step: "welcome",
    dictText: "",
    cleanText: MESSY_TEXT,
    clean: "messy",
  };
}

function tourModel() {
  const m = S.models.find((x) => x.id === S.settings.model);
  const d = m?.download || {};
  return { m, d, pct: d.total ? Math.round((d.done / d.total) * 100) : 0 };
}

/** Starts the speech model download in the background, once. */
function tourEnsureModel() {
  const { m, d } = tourModel();
  if (!m || m.ready || d.active || ui.tour.downloadAsked) return;
  ui.tour.downloadAsked = true;
  invoke("download_model", { id: m.id }).catch(() => {});
}

function tourStatus() {
  const where = pillWhere(S.settings.pillPosition);
  if (S.phase === "recording") return `<span class="live-dot"></span> Listening… The small bar ${where} means Wirl can hear you.`;
  if (S.phase === "processing") return `<span class="spinner small-spin"></span> Tidying up your words…`;
  return `While Wirl listens, a small bar appears ${where}.`;
}

function tourDictFeedback() {
  const text = ui.tour.dictText;
  if (!text.trim()) return "";
  const summary = S.apiKey.saved && S.settings.cleanupEnabled ? dictationSummary(text) : "";
  const noKey = !S.apiKey.saved
    ? `<div class="muted small" style="margin-top:6px">Without an API key, Wirl types exactly what it heard. You can add your key any time in Setup.</div>`
    : "";
  return `<div class="row"><span class="ok-text">Nice, that's dictation!</span><a class="link small" data-act="tour-dict-again">Try again</a></div>${
    summary ? `<div class="muted" style="margin-top:4px">${esc(summary)}</div>` : ""
  }${noKey}`;
}

function tourCleanFeedback() {
  const key = `<kbd>${esc(S.hotkeys.cleanSelection)}</kbd>`;
  const again = `<a class="link small" data-act="tour-clean-reset">Start over</a>`;
  if (ui.tour.clean === "cleaned")
    return `<div class="row"><span class="ok-text">Tidied!</span>${again}</div><div class="muted" style="margin-top:4px">Changed your mind? Press ${key} again right away to undo.</div>`;
  if (ui.tour.clean === "undone")
    return `<div class="row"><span class="ok-text">Back to how it was.</span>${again}</div><div class="muted" style="margin-top:4px">Undo works right after any clean-up, in any app.</div>`;
  return "";
}

function tourFooter({ back = true, next = "Next", nextPrimary = true, skip = true } = {}) {
  return `<div class="tour-footer">
    ${back ? `<button class="ghost" data-act="tour-back">Back</button>` : "<span></span>"}
    <div class="row">
      ${skip ? `<a class="link muted small" data-act="tour-skip">Skip the tour</a>` : ""}
      <button class="${nextPrimary ? "primary" : ""}" data-act="tour-next">${next}</button>
    </div>
  </div>`;
}

function tourDots() {
  const i = ui.tour.steps.indexOf(ui.tour.step);
  return `<div class="tour-dots">${ui.tour.steps.map((_, n) => `<span class="${n === i ? "on" : n < i ? "done" : ""}"></span>`).join("")}</div>`;
}

function tourWelcome() {
  const f = iconFile();
  return `${f ? `<img class="tour-logo" src="assets/icons/${encodeURIComponent(f)}" alt="">` : ""}
    <h1>Welcome to Wirl</h1>
    <p class="lead">Wirl helps you write in two ways. Let's try both. It takes about 2 minutes.</p>
    <div class="tour-two">
      <div class="card"><div class="tour-icon">${MIC_SVG}</div><h3>Talk, and it types for you</h3>
        <div class="muted">Speak naturally. Wirl writes it out, tidied up, wherever you're typing.</div></div>
      <div class="card"><div class="tour-icon">${SPARK_SVG}</div><h3>Tidy up any text</h3>
        <div class="muted">Select messy text and Wirl cleans it up in place.</div></div>
    </div>
    ${tourFooter({ back: false, next: "Let's go", skip: false })}
    <div class="center"><a class="link muted small" data-act="tour-skip">Skip for now</a></div>`;
}

function tourReady() {
  const { m, d, pct } = tourModel();
  const model = !m
    ? ""
    : m.ready
      ? `<div class="ok-text small">Speech model is ready.</div>`
      : `<div class="muted small">Getting the speech model ready in the background (${esc(m.sizeLabel)}, one time)… <span data-progress-label="${esc(m.file)}">${pct}%</span></div>
         <div class="progress"><div data-progress="${esc(m.file)}" style="width:${pct}%"></div></div>${d.error ? `<div class="error-text">${esc(d.error)}</div>` : ""}`;
  const done = S.setup.microphone && S.setup.apiKey;
  return `<h1>First, two quick things</h1>
    <p class="lead">Wirl needs to hear you, and it uses Claude to tidy your words.</p>
    <div class="card">
      ${micItem()}
      ${checkItem(S.setup.apiKey, "API key", S.setup.apiKey ? "Saved. Only your text is sent to Anthropic; audio stays on this computer." : apiKeyBody(), "")}
    </div>
    <div class="card">${model}</div>
    ${tourFooter({ next: done ? "Continue" : "Continue anyway", nextPrimary: done })}`;
}

function tourDictate() {
  const { m, pct } = tourModel();
  const ins = dictateInstruction(S.hotkeys.dictate, S.settings.holdToTalk);
  const notReady = m && !m.ready;
  return `<h1>Try talking</h1>
    <p class="lead">${ins.before} <kbd>${esc(S.hotkeys.dictate)}</kbd> ${ins.after}</p>
    <div class="row tour-choice">
      <span class="muted small grow">How do you like to talk? You can change this later in Hotkeys.</span>
      <div class="segmented small-seg">
        <button data-act="tour-hold" data-id="1" class="${S.settings.holdToTalk ? "on" : ""}">Hold the keys</button>
        <button data-act="tour-hold" data-id="0" class="${S.settings.holdToTalk ? "" : "on"}">Tap on, tap off</button>
      </div>
    </div>
    <div class="card tour-script"><div class="muted small">Read this out loud</div><div class="script">“${esc(DICTATION_SCRIPT)}”</div></div>
    ${
      notReady
        ? `<div class="warning-box">Wirl is still getting its speech model ready (<span data-progress-label="${esc(m.file)}">${pct}%</span>). You can try as soon as it's done.</div>`
        : ""
    }
    <textarea id="tour-dict" class="tour-box" placeholder="Your words appear here">${esc(ui.tour.dictText)}</textarea>
    <div id="tour-status" class="muted small tour-status">${tourStatus()}</div>
    <div id="tour-feedback">${tourDictFeedback()}</div>
    ${tourFooter({ nextPrimary: !!ui.tour.dictText.trim() })}`;
}

function tourClean() {
  const noKey = !S.apiKey.saved;
  return `<h1>Now tidy up some text</h1>
    <p class="lead">Select the text in the box, then press <kbd>${esc(S.hotkeys.cleanSelection)}</kbd>.</p>
    ${
      noKey
        ? `<div class="warning-box">Tidying up uses Claude, so it needs your API key. <a class="link" data-act="tour-to-ready">Add your key</a></div>`
        : ""
    }
    <textarea id="tour-clean" class="tour-box tall">${esc(ui.tour.cleanText)}</textarea>
    <div style="margin-top:8px"><button class="ghost" data-act="tour-select">Select it for me</button></div>
    <div id="tour-feedback">${tourCleanFeedback()}</div>
    ${tourFooter({ nextPrimary: ui.tour.clean !== "messy" })}`;
}

function tourDone() {
  return `<h1>You're all set</h1>
    <p class="lead">Wirl works in any app: email, Word, chat, anywhere you type.</p>
    <div class="card stack">${shortcutRows()}
      <div class="muted small">Changed your mind while talking? <kbd>${esc(S.hotkeys.cancel)}</kbd> throws the recording away.</div>
    </div>
    <div class="card"><h3>Wirl lives in your system tray</h3>
      <div class="muted">Look for its icon near the clock. Click it any time to open these settings.</div></div>
    <h2>When you're ready</h2>
    <div class="tour-next-steps">
      <a class="card clickable" data-act="tour-finish" data-id="dictionary"><h3>Add names to your dictionary</h3><div class="muted small">So Wirl always spells them right.</div></a>
      <a class="card clickable" data-act="tour-finish" data-id="dictionary"><h3>Pair your phone</h3><div class="muted small">Share your dictionary with Wirl on Android.</div></a>
      <a class="card clickable" data-act="tour-finish" data-id="indicator"><h3>Pick your icon</h3><div class="muted small">25 to choose from.</div></a>
    </div>
    <div class="tour-footer"><button class="ghost" data-act="tour-back">Back</button><button class="primary" data-act="tour-finish" data-id="home">Start using Wirl</button></div>`;
}

function pageTour() {
  if (!ui.tour) initTour();
  if (ui.tour.step !== "welcome") tourEnsureModel();
  const body = { welcome: tourWelcome, ready: tourReady, dictate: tourDictate, clean: tourClean, done: tourDone }[ui.tour.step]();
  return `<div class="page tour">${tourDots()}${body}</div>`;
}

function tourMove(delta) {
  const steps = ui.tour.steps;
  let i = steps.indexOf(ui.tour.step);
  // "ready" can be visited from the clean-up step even when it wasn't in the list.
  if (i < 0) i = steps.indexOf("clean") - (delta > 0 ? 1 : 0);
  const next = steps[Math.min(steps.length - 1, Math.max(0, i + delta))];
  ui.tour.step = next;
  document.activeElement?.blur();
  main.scrollTop = 0;
  render();
}

async function finishTour(target) {
  ui.tour = null;
  if (!S.settings.firstRunDone) await invoke("mark_first_run_done");
  go(target || "home");
}

// ---------- render ----------

function render() {
  if (!S) return;
  if (focusInMain()) {
    renderPending = true;
    return;
  }
  renderPending = false;
  document.body.classList.toggle("touring", page === "tour");
  document.querySelectorAll("#nav a").forEach((a) => a.classList.toggle("active", a.dataset.page === page));
  document.getElementById("setup-dot").hidden = attentionCount() === 0;
  const brand = document.getElementById("brand-icon");
  const f = iconFile();
  if (f) brand.src = `assets/icons/${encodeURIComponent(f)}`;
  const scroll = main.scrollTop;
  const fn = { home: pageHome, dictation: pageDictation, cleanup: pageCleanup, dictionary: pageDictionary, hotkeys: pageHotkeys, indicator: pageIndicator, setup: pageSetup, about: pageAbout, tour: pageTour }[page];
  main.innerHTML = fn();
  main.scrollTop = scroll;
  afterRender();
}

function afterRender() {
  clearInterval(previewTimer);
  if (page === "indicator") startPreview();
  const notes = document.getElementById("style-notes");
  if (notes) {
    let t;
    notes.addEventListener("input", () => {
      clearTimeout(t);
      t = setTimeout(() => save({ styleNotes: notes.value }), 400);
    });
    notes.addEventListener("blur", () => save({ styleNotes: notes.value }).then(flushPending));
  }
  const search = document.getElementById("dict-search");
  if (search) {
    search.addEventListener("input", () => {
      ui.search = search.value;
      const pos = search.selectionStart;
      search.blur();
      render();
      const s2 = document.getElementById("dict-search");
      s2.focus();
      s2.setSelectionRange(pos, pos);
    });
  }
  const range = document.getElementById("size-range");
  if (range) range.addEventListener("change", () => save({ pillSize: SIZES[Number(range.value)] }));
  const tourDict = document.getElementById("tour-dict");
  if (tourDict) {
    tourDict.addEventListener("input", () => {
      ui.tour.dictText = tourDict.value;
      document.getElementById("tour-feedback").innerHTML = tourDictFeedback();
      main.querySelector('[data-act="tour-next"]')?.classList.toggle("primary", !!tourDict.value.trim());
    });
    if (!ui.tour.dictText) tourDict.focus();
  }
  const tourClean = document.getElementById("tour-clean");
  if (tourClean) {
    tourClean.addEventListener("input", () => {
      ui.tour.cleanText = tourClean.value;
      ui.tour.clean = cleanState(ui.tour.clean, tourClean.value);
      document.getElementById("tour-feedback").innerHTML = tourCleanFeedback();
      main.querySelector('[data-act="tour-next"]')?.classList.toggle("primary", ui.tour.clean !== "messy");
    });
  }
  const test = document.getElementById("test-box");
  if (test) test.addEventListener("input", () => (ui.testText = test.value));
  main.querySelectorAll("input, textarea").forEach((el) => el.addEventListener("blur", () => setTimeout(flushPending, 50)));
  const form = document.getElementById("dict-form");
  if (form) {
    form.querySelector("#f-term").focus();
    form.addEventListener("keydown", (e) => {
      if (e.key === "Enter") act("dict-save");
      if (e.key === "Escape") act("dict-cancel");
    });
  }
  const key = document.getElementById("api-key");
  if (key) key.addEventListener("keydown", (e) => e.key === "Enter" && act("api-save"));
}

function flushPending() {
  if (renderPending && !focusInMain()) render();
}

function startPreview() {
  const el = document.getElementById("preview");
  if (!el) return;
  const s = S.settings;
  const states = [
    { kind: "recording" },
    { kind: "processing" },
    { kind: "inserted" },
    { kind: "cleaned", undoHint: `${S.hotkeys.cleanSelection} to undo` },
  ];
  let i = 0;
  const wave = makeWave();
  let t = 0;
  const show = () => renderPill(el, states[i], { icon: iconFile(), size: s.pillSize });
  show();
  previewTimer = setInterval(() => {
    t += 1;
    if (t % 28 === 0) {
      i = (i + 1) % states.length;
      show();
    }
    if (states[i].kind === "recording") wave(el, 0.02 + Math.abs(Math.sin(t / 3)) * 0.05 * Math.random() * 2);
  }, 80);
}

function go(p) {
  if (!PAGES.includes(p)) return;
  if (ui.capture) endCapture();
  page = p;
  ui.confirmReset = false;
  ui.confirmDisconnect = false;
  ui.confirmDeleteModel = null;
  if (p === "dictionary") invoke("sync_now").catch(() => {});
  history.replaceState(null, "", `#${p}`);
  main.scrollTop = 0;
  renderPending = false;
  if (document.activeElement) document.activeElement.blur();
  render();
}

// ---------- hotkey capture ----------

async function startCapture(action) {
  ui.capture = action;
  ui.keyMsg[action] = null;
  document.activeElement?.blur();
  render();
  await invoke("hotkey_capture", { active: true });
}

async function endCapture() {
  ui.capture = null;
  await invoke("hotkey_capture", { active: false });
  await load();
}

window.addEventListener(
  "keydown",
  async (e) => {
    if (!ui.capture) return;
    e.preventDefault();
    e.stopPropagation();
    const action = ui.capture;
    const r = await invoke("hotkey_key", {
      action,
      ctrl: e.ctrlKey,
      alt: e.altKey,
      shift: e.shiftKey,
      win: e.metaKey,
      code: e.code,
    });
    if (r.status === "waiting") return;
    if (r.status === "rejected") {
      ui.keyMsg[action] = { error: r.error };
      ui.capture = null;
      await invoke("hotkey_capture", { active: false });
      await load();
      return;
    }
    ui.keyMsg[action] = { warning: r.warning };
    await endCapture();
  },
  true,
);
window.addEventListener("blur", () => ui.capture && endCapture());

// ---------- actions ----------

async function act(name, el) {
  const id = el?.dataset.id;
  try {
    switch (name) {
      case "use-model":
        await invoke("use_model", { id });
        break;
      case "tick": {
        const m = S.models.find((x) => x.id === id);
        await invoke("set_quick_switch", { id, on: !m?.ticked });
        break;
      }
      case "download":
        await invoke("download_model", { id });
        break;
      case "cancel-download":
        await invoke("cancel_download", { id });
        break;
      case "delete-model":
        ui.confirmDeleteModel = id;
        render();
        break;
      case "delete-model-no":
        ui.confirmDeleteModel = null;
        render();
        break;
      case "delete-model-yes":
        ui.confirmDeleteModel = null;
        await invoke("delete_model", { id });
        break;
      case "position":
        await save({ pillPosition: id });
        break;
      case "icon":
        await save({ icon: id });
        break;
      case "dict-add":
        ui.adding = true;
        ui.editing = null;
        ui.dictError = "";
        render();
        break;
      case "dict-edit":
        ui.editing = id;
        ui.adding = false;
        ui.dictError = "";
        render();
        break;
      case "dict-cancel":
        ui.adding = false;
        ui.editing = null;
        document.activeElement?.blur();
        render();
        break;
      case "dict-save": {
        const term = document.getElementById("f-term").value;
        const soundsLike = document.getElementById("f-sounds").value;
        const note = document.getElementById("f-note").value;
        try {
          if (ui.editing) await invoke("dict_edit", { id: ui.editing, term, soundsLike, note });
          else await invoke("dict_add", { term, soundsLike, note });
          ui.adding = false;
          ui.editing = null;
          ui.dictError = "";
          document.activeElement?.blur();
          await load();
        } catch (err) {
          ui.dictError = String(err);
          document.activeElement?.blur();
          render();
        }
        break;
      }
      case "dict-delete":
        await invoke("dict_delete", { id });
        break;
      case "dict-import": {
        const r = await invoke("dict_import");
        if (r) toast(r);
        break;
      }
      case "dict-export": {
        const r = await invoke("dict_export");
        if (r) toast(r);
        break;
      }
      case "pair":
        ui.pairing = await invoke("pair_begin");
        render();
        break;
      case "pair-from-setup":
        go("dictionary");
        await act("pair");
        break;
      case "pair-close":
        ui.pairing = null;
        render();
        break;
      case "sync-now":
        await invoke("sync_now");
        break;
      case "disconnect-all":
        if (!ui.confirmDisconnect) {
          ui.confirmDisconnect = true;
          render();
        } else {
          ui.confirmDisconnect = false;
          ui.pairing = null;
          await invoke("disconnect_all_phones");
          toast("All phones disconnected. Pair again with a new code.");
        }
        break;
      case "capture":
        if (ui.capture === id) await endCapture();
        else await startCapture(id);
        break;
      case "hotkeys-reset":
        ui.keyMsg = {};
        await invoke("hotkeys_reset");
        break;
      case "api-save": {
        const key = document.getElementById("api-key").value;
        ui.apiBusy = true;
        ui.apiError = "";
        document.activeElement?.blur();
        render();
        try {
          const hint = await invoke("set_api_key", { key });
          ui.apiEditing = false;
          toast(hint);
        } catch (err) {
          ui.apiError = String(err);
        }
        ui.apiBusy = false;
        await load();
        break;
      }
      case "api-change":
        ui.apiEditing = true;
        render();
        break;
      case "api-remove":
        await invoke("clear_api_key");
        break;
      case "open":
        await invoke("open_link", { url: el.dataset.url });
        break;
      case "test": {
        // Keep focus in the box so the result is pasted there.
        document.getElementById("test-box")?.focus();
        await invoke("test_dictation");
        break;
      }
      case "tour-start":
        ui.tour = null;
        go("tour");
        break;
      case "tour-next":
        tourMove(1);
        break;
      case "tour-back":
        tourMove(-1);
        break;
      case "tour-skip":
        await finishTour("home");
        break;
      case "tour-finish":
        await finishTour(id);
        break;
      case "tour-to-ready":
        ui.tour.step = "ready";
        render();
        break;
      case "tour-hold":
        await save({ holdToTalk: id === "1" });
        break;
      case "tour-dict-again":
        ui.tour.dictText = "";
        render();
        break;
      case "tour-select": {
        const box = document.getElementById("tour-clean");
        box?.focus();
        box?.select();
        break;
      }
      case "tour-clean-reset":
        ui.tour.cleanText = MESSY_TEXT;
        ui.tour.clean = "messy";
        render();
        break;
      case "version":
        ui.devClicks += 1;
        if (ui.devClicks >= 7 && !S.settings.developerOptions) {
          await invoke("reveal_developer_options");
          toast("Developer options are on");
        }
        break;
      case "check-updates":
        await invoke("check_updates");
        break;
      case "restart-update":
        await invoke("restart_to_update");
        break;
      case "reset":
        if (!ui.confirmReset) {
          ui.confirmReset = true;
          render();
        } else {
          ui.confirmReset = false;
          await invoke("reset_defaults");
          toast("Settings reset");
        }
        break;
    }
  } catch (err) {
    toast(String(err));
  }
}

main.addEventListener("click", (e) => {
  const goEl = e.target.closest("[data-go]");
  if (goEl) return go(goEl.dataset.go);
  const a = e.target.closest("[data-act]");
  if (!a) return;
  act(a.dataset.act, a);
});

main.addEventListener("change", (e) => {
  const el = e.target;
  const key = el.dataset?.setting;
  if (!key) return;
  const value = el.type === "checkbox" ? el.checked : el.value;
  save({ [key]: value });
});

document.getElementById("nav").addEventListener("click", (e) => {
  const a = e.target.closest("a[data-page]");
  if (a) go(a.dataset.page);
});

// ---------- events from the app ----------

listen("wirl://state", load);
listen("wirl://navigate", (e) => go(e.payload));
listen("wirl://paired", (e) => {
  ui.pairing = null;
  toast(`Paired with ${e.payload}`);
  load();
});
listen("wirl://download", (e) => {
  // Downloads are per file: Cantonese and Mandarin share one.
  const { file, done, total } = e.payload;
  const pct = total ? Math.round((done / total) * 100) : 0;
  for (const m of S?.models.filter((x) => x.file === file) || []) {
    // The SenseVoice model is unpacked after downloading; the state update says so.
    m.download = { ...(m.download || {}), done, total, active: done < total || !!m.download?.unpacking };
  }
  document.querySelectorAll(`[data-progress="${file}"]`).forEach((b) => (b.style.width = `${pct}%`));
  document.querySelectorAll(`[data-progress-label="${file}"]`).forEach((b) => (b.textContent = `${pct}%`));
});

await load();
// On first launch the tour marks the first run done when it's finished or
// skipped, so closing the window halfway brings it back next time.
if (!S.settings.firstRunDone && page !== "tour") await invoke("mark_first_run_done");
