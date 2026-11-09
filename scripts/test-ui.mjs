// Tests for the UI logic that doesn't need a browser (node scripts/test-ui.mjs).
import assert from "node:assert/strict";
import {
  DICTATION_SCRIPT,
  MESSY_TEXT,
  tourSteps,
  dictationNotes,
  dictationSummary,
  sameText,
  cleanState,
  dictateInstruction,
  pillWhere,
} from "../ui/tour-logic.js";

const tests = {
  "setup step only when something is missing"() {
    assert.deepEqual(tourSteps({ microphone: true, apiKey: true }), ["welcome", "dictate", "clean", "done"]);
    assert.deepEqual(tourSteps({ microphone: true, apiKey: false }), ["welcome", "ready", "dictate", "clean", "done"]);
    assert.deepEqual(tourSteps({ microphone: false, apiKey: true })[1], "ready");
  },
  "practice script has a filler word and a correction"() {
    assert.match(DICTATION_SCRIPT, /\bum\b/);
    assert.match(DICTATION_SCRIPT, /make that Wednesday/);
  },
  "notes describe only what happened"() {
    const clean = "Hey, just checking we're still on for lunch Wednesday. Let me know if that works.";
    assert.deepEqual(dictationNotes(clean), ["dropped the “um”", "kept your change to Wednesday"]);
    assert.equal(dictationSummary(clean), "Wirl dropped the “um” and kept your change to Wednesday.");
    const raw = "Hey um just checking we're still on for lunch Tuesday actually make that Wednesday";
    assert.deepEqual(dictationNotes(raw), []);
    assert.equal(dictationSummary(raw), "");
    assert.deepEqual(dictationNotes("Something else entirely."), ["dropped the “um”"]);
    assert.deepEqual(dictationNotes("  "), []);
  },
  "clean-up practice states"() {
    assert.equal(cleanState("messy", MESSY_TEXT), "messy");
    assert.equal(cleanState("messy", `  ${MESSY_TEXT}\n`), "messy", "whitespace doesn't count");
    assert.equal(cleanState("messy", "Hi all, quick update: the meeting has moved to Thursday at 3."), "cleaned");
    assert.equal(cleanState("cleaned", MESSY_TEXT), "undone");
    assert.equal(cleanState("undone", "Hi all, tidy again."), "cleaned");
    assert.equal(cleanState("cleaned", ""), "cleaned", "an empty box mid-paste doesn't reset");
  },
  "sameText ignores spacing"() {
    assert.ok(sameText("a  b\n", "a b"));
    assert.ok(!sameText("a b", "a c"));
  },
  "instructions follow hold-to-talk"() {
    assert.match(dictateInstruction("Ctrl+Alt+Space", true).after, /Let go/);
    assert.match(dictateInstruction("Ctrl+Alt+Space", false).after, /again/);
  },
  "pill position in words"() {
    assert.equal(pillWhere("bottomCenter"), "at the bottom of your screen");
    assert.equal(pillWhere("topCenter"), "at the top of your screen");
    assert.equal(pillWhere("nearMouse"), "next to your mouse");
  },
};

let failed = 0;
for (const [name, fn] of Object.entries(tests)) {
  try {
    fn();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.error(`FAIL ${name}\n${e.message}`);
  }
}
process.exit(failed ? 1 : 0);
