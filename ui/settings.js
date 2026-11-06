import { renderPill, makeWave } from "./pill-view.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const main = document.getElementById("main");
const PAGES = ["home", "dictation", "cleanup", "dictionary", "hotkeys", "indicator", "setup", "about"];

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

function modelCard(m) {
  const s = S.settings;
  const selected = s.model === m.id;
  const d = m.download || {};
  let right;
  if (m.ready) right = `<span class="pill-tag ok">Ready</span>`;
  else if (d.active) right = `<button data-act="cancel-download" data-id="${m.id}">Cancel</button>`;
  else right = `<button data-act="download" data-id="${m.id}">Download · ${esc(m.sizeLabel)}</button>`;
  const pct = d.total ? Math.round((d.done / d.total) * 100) : 0;
  return `<div class="card clickable ${selected ? "selected" : ""}" data-act="select-model" data-id="${m.id}">
    <div class="row">
      <div class="grow"><h3>${esc(m.name)}${selected ? ` <span class="muted small">· selected</span>` : ""}</h3>
        <div class="muted">${esc(m.description)}</div></div>
      ${right}
    </div>
    ${d.active ? `<div class="progress"><div data-progress="${m.id}" style="width:${pct}%"></div></div><div class="muted small" data-progress-label="${m.id}" style="margin-top:6px">${pct}%</div>` : ""}
    ${d.error ? `<div class="error-text">${esc(d.error)}</div>` : ""}
  </div>`;
}

function pageDictation() {
  const s = S.settings;
  const model = S.models.find((m) => m.id === s.model);
  const englishOnly = model?.englishOnly;
  return `<div class="page">
    <h1>Dictation</h1>
    <p class="lead">Speech is turned into text on this computer. Models download once when you pick them.</p>
    <h2>Speech model</h2>
    ${S.models.map(modelCard).join("")}
    <h2>Language</h2>
    <div class="card">
      <select data-setting="language" ${englishOnly ? "disabled" : ""}>
        ${S.languages.map(([c, l]) => `<option value="${c}" ${(englishOnly ? "en" : s.language) === c ? "selected" : ""}>${esc(l)}</option>`).join("")}
      </select>
      ${englishOnly ? `<div class="muted small" style="margin-top:8px">This model understands English only. Choose Multilingual for other languages.</div>` : ""}
    </div>
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
    <input type="text" id="f-term" placeholder="Term, e.g. Sela" value="${esc(e?.term)}">
    <input type="text" id="f-sounds" placeholder="Sounds like, separated by commas, e.g. salad, sella" value="${esc((e?.soundsLike || []).join(", "))}">
    <input type="text" id="f-note" placeholder="Context note, e.g. my daughter's name" value="${esc(e?.note)}">
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
            ${e.soundsLike.length ? `<div class="chips">${e.soundsLike.map((x) => `<span class="chip">${esc(x)}</span>`).join("")}</div>` : ""}
            ${e.note ? `<div class="muted small" style="margin-top:6px">${esc(e.note)}</div>` : ""}</div>
          <button class="ghost" data-act="dict-edit" data-id="${e.id}">Edit</button>
          <button class="ghost" data-act="dict-delete" data-id="${e.id}">Delete</button>
        </div>`,
              )
              .join("")
      }
    </div>
    <h2>Phone sync</h2>
    <div class="card">
      ${
        S.phones.length
          ? S.phones
              .map(
                (p) => `<div class="row" style="padding:6px 0"><div class="grow"><div class="term">${esc(p.name)}</div>
                <div class="muted small">Synced ${ago(p.lastSync)}</div></div>
                <button class="ghost" data-act="unpair" data-id="${p.phoneId}">Unpair</button></div>`,
              )
              .join('<div class="divider"></div>')
          : `<div class="muted">No phone paired yet.</div>`
      }
      <div class="divider"></div>
      ${
        ui.pairing
          ? `<div class="row" style="align-items:flex-start;gap:22px">
              <div class="qr">${ui.pairing.svg}</div>
              <div class="grow stack">
                <div>On your phone: Wirl › Dictionary › Pair with computer, then scan. Both devices sync when on the same Wi-Fi.</div>
                <div class="muted small">If Windows Firewall asks, allow Wirl on Private networks.</div>
                <button class="ghost" data-act="pair-cancel">Done</button>
              </div></div>`
          : `<button class="primary" data-act="pair">Pair with phone</button>`
      }
    </div>
  </div>`;
}

const HOTKEY_ROWS = [
  ["dictate", "Dictate", "Tap to start and stop. With Hold to talk on, hold it while you speak."],
  ["cancel", "Cancel", "Only while recording: throws the recording away."],
  ["cleanSelection", "Clean up selection", "Cleans the selected text. Press again right away to undo."],
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

function pageSetup() {
  const st = S.setup;
  const model = S.models.find((m) => m.id === S.settings.model);
  const d = model?.download || {};
  const pct = d.total ? Math.round((d.done / d.total) * 100) : 0;
  const keyBody =
    S.apiKey.saved && !ui.apiEditing
      ? `${esc(S.apiKey.hint)} · <a class="link" data-act="api-change">Change</a> · <a class="link" data-act="api-remove">Remove</a>`
      : `Each person uses their own Anthropic API key. It's stored in Windows Credential Manager.
         <div class="row" style="margin-top:10px"><input type="password" id="api-key" class="grow" placeholder="sk-ant-..." autocomplete="off">
         <button class="primary" data-act="api-save" ${ui.apiBusy ? "disabled" : ""}>${ui.apiBusy ? "Checking…" : "Save"}</button></div>
         ${ui.apiError ? `<div class="error-text">${esc(ui.apiError)}</div>` : ""}
         <div class="small" style="margin-top:8px"><a class="link" data-act="open" data-url="https://console.anthropic.com/settings/keys">Get a key from the Anthropic Console</a></div>`;
  const recording = S.phase === "recording";
  return `<div class="page">
    <h1>Setup</h1>
    <p class="lead">A few things Wirl needs. Green means done.</p>
    <div class="card">
      ${checkItem(st.apiKey, "API key", keyBody, "")}
      ${checkItem(
        st.microphone,
        "Microphone access",
        st.microphone ? "Wirl can hear your microphone." : "Wirl can't use a microphone. Plug one in, or allow desktop apps to use it in Windows privacy settings.",
        `<button class="warn" data-act="open" data-url="ms-settings:privacy-microphone">Fix</button>`,
      )}
      ${checkItem(
        st.model,
        "Speech model",
        st.model
          ? `${esc(model.name)} model is ready.`
          : d.active
            ? `Downloading ${esc(model.name)}… ${pct}%<div class="progress"><div data-progress="${model.id}" style="width:${pct}%"></div></div>`
            : `The ${esc(model.name)} model (${esc(model.sizeLabel)}) needs to download once.${d.error ? `<div class="error-text">${esc(d.error)}</div>` : ""}`,
        d.active ? "" : `<button class="warn" data-act="download" data-id="${model.id}">Fix</button>`,
      )}
      ${checkItem(
        st.paired,
        "Phone pairing <span class='muted small'>(optional)</span>",
        st.paired
          ? `Paired with ${esc(S.phones.map((p) => p.name).join(", "))}.`
          : "Share your dictionary with Wirl on Android. When Windows Firewall asks, allow Wirl on Private networks.",
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

// ---------- render ----------

function render() {
  if (!S) return;
  if (focusInMain()) {
    renderPending = true;
    return;
  }
  renderPending = false;
  document.querySelectorAll("#nav a").forEach((a) => a.classList.toggle("active", a.dataset.page === page));
  document.getElementById("setup-dot").hidden = attentionCount() === 0;
  const brand = document.getElementById("brand-icon");
  const f = iconFile();
  if (f) brand.src = `assets/icons/${encodeURIComponent(f)}`;
  const scroll = main.scrollTop;
  const fn = { home: pageHome, dictation: pageDictation, cleanup: pageCleanup, dictionary: pageDictionary, hotkeys: pageHotkeys, indicator: pageIndicator, setup: pageSetup, about: pageAbout }[page];
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
    { kind: "recording", cancelHint: `${S.hotkeys.cancel} to cancel` },
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
      case "select-model":
        if (S.settings.model !== id) await save({ model: id });
        break;
      case "download":
        await invoke("download_model", { id });
        break;
      case "cancel-download":
        await invoke("cancel_download", { id });
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
      case "pair-cancel":
        ui.pairing = null;
        await invoke("pair_cancel");
        render();
        break;
      case "unpair":
        await invoke("unpair", { phoneId: id });
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
  // Buttons inside a clickable card shouldn't also trigger the card.
  if (a.dataset.act === "select-model" && e.target.closest("button")) return;
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
  const { id, done, total } = e.payload;
  const pct = total ? Math.round((done / total) * 100) : 0;
  const m = S?.models.find((x) => x.id === id);
  if (m) m.download = { ...(m.download || {}), done, total, active: done < total };
  document.querySelectorAll(`[data-progress="${id}"]`).forEach((b) => (b.style.width = `${pct}%`));
  document.querySelectorAll(`[data-progress-label="${id}"]`).forEach((b) => (b.textContent = `${pct}%`));
});

await load();
if (!S.settings.firstRunDone) await invoke("mark_first_run_done");
