// The welcome tour's content and rules, kept free of the DOM so they can be
// tested with plain Node (scripts/test-ui.mjs).

/** What the user reads aloud in the dictation practice. */
export const DICTATION_SCRIPT =
  "Hey, um, just checking we're still on for lunch Tuesday, actually, make that Wednesday. Let me know if that works.";

/** The messy text in the clean-up practice. */
export const MESSY_TEXT =
  "hi all, quick update the meeting is moved to thursday at 3 i think thats better for everyone since most people said tuesday doesnt work, let me know if theres a problem thx";

/** Step ids, in order. "ready" only appears when something is missing. */
export function tourSteps(setup) {
  const needsSetup = !setup.microphone || !setup.apiKey;
  return ["welcome", ...(needsSetup ? ["ready"] : []), "dictate", "clean", "done"];
}

/**
 * Plain notes about what cleanup did to the practice sentence. Only states
 * what actually happened, since speech varies.
 */
export function dictationNotes(text) {
  const t = String(text || "");
  if (!t.trim()) return [];
  const notes = [];
  const lower = t.toLowerCase();
  const saidWednesday = /\bwednesday\b/.test(lower);
  const keptTuesday = /\btuesday\b/.test(lower);
  if (!/\b(um+|uh+|erm)\b/.test(lower)) notes.push("dropped the “um”");
  if (saidWednesday && !keptTuesday) notes.push("kept your change to Wednesday");
  return notes;
}

/** One friendly sentence built from dictationNotes(), or "" if there's nothing to point out. */
export function dictationSummary(text) {
  const n = dictationNotes(text);
  if (n.length === 0) return "";
  return `Wirl ${n.join(" and ")}.`;
}

/** Whitespace-insensitive comparison for the practice boxes. */
export function sameText(a, b) {
  const norm = (s) => String(s || "").replace(/\s+/g, " ").trim();
  return norm(a) === norm(b);
}

/**
 * Clean-up practice: given the previous state and the box's new text,
 * returns "messy" (untouched), "cleaned" or "undone".
 */
export function cleanState(prev, text, messy = MESSY_TEXT) {
  if (sameText(text, messy)) return prev === "cleaned" || prev === "undone" ? "undone" : "messy";
  if (!String(text || "").trim()) return prev;
  return "cleaned";
}

/** Instruction for the dictation step, matching the user's hotkey and style. */
export function dictateInstruction(hotkey, holdToTalk) {
  return holdToTalk
    ? { before: "Click in the box, then hold", after: "while you read the sentence out loud. Let go when you're done." }
    : { before: "Click in the box, press", after: "and read the sentence out loud. Press it again when you're done." };
}

/** Where the pill appears, in words. */
export function pillWhere(position) {
  if (position === "topCenter") return "at the top of your screen";
  if (position === "nearMouse") return "next to your mouse";
  return "at the bottom of your screen";
}
