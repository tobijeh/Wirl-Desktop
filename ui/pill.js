import { renderPill, makeWave } from "./pill-view.js";

const { listen } = window.__TAURI__.event;
const el = document.getElementById("pill");
const wave = makeWave();

listen("wirl://pill", (e) => {
  const { state, icon, size } = e.payload;
  renderPill(el, state, { icon, size });
});

listen("wirl://pill-fade", () => el.classList.add("fade"));

listen("wirl://level", (e) => wave(el, e.payload));
