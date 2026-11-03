import { test } from "node:test";
import assert from "node:assert/strict";
import {
  addable,
  INPUT_METHODS,
  inputSourcesOf,
  layoutName,
  layoutShortName,
  parseInputSources,
  parseInputSourcesPreference,
  removable,
  selectedAfterRemoving,
  withAdded,
  withRemoved,
} from "../src/input-sources.ts";
import { messagesFor } from "../src/messages.ts";

const told = {
  layout: "es",
  current: "pinyin",
  sources: ["layout", "pinyin"],
  available: ["pinyin", "japanese", "korean"],
};

test("the input methods are the preferences' (crates/oxyos-preferences, INPUT_METHODS)", () => {
  assert.deepEqual([...INPUT_METHODS], ["pinyin", "japanese", "korean"]);
});

test("the session's state is read strictly", () => {
  assert.deepEqual(parseInputSources(told), told);
  assert.deepEqual(parseInputSources({ ...told, available: [] }), { ...told, available: [] });
  // Not the session's: the layout not first, or twice; an unknown or repeated method; the current
  // not among the sources; a part missing.
  for (const bad of [
    { ...told, sources: ["pinyin", "layout"] },
    { ...told, sources: ["layout", "layout"] },
    { ...told, sources: ["layout", "pinyin", "pinyin"] },
    { ...told, sources: ["layout", "zhuyin"] },
    { ...told, available: ["pinyin", "pinyin"] },
    { ...told, available: ["rime"] },
    { ...told, current: "korean" },
    { ...told, layout: undefined },
    { ...told, sources: undefined },
    null,
    "layout",
  ]) {
    assert.equal(parseInputSources(bad), null, JSON.stringify(bad));
  }
});

test("without a session, the preferences' list after the layout", () => {
  assert.deepEqual(inputSourcesOf("us", ["korean", "pinyin", "korean", "rime"], "pinyin", INPUT_METHODS), {
    layout: "us",
    current: "pinyin",
    sources: ["layout", "korean", "pinyin"],
    available: ["pinyin", "japanese", "korean"],
  });
  // A current one not in the list: the layout.
  assert.equal(inputSourcesOf("us", ["korean"], "japanese", []).current, "layout");
});

test("the preference's text, as the session keeps it", () => {
  assert.deepEqual(parseInputSourcesPreference("pinyin,japanese"), ["pinyin", "japanese"]);
  assert.deepEqual(parseInputSourcesPreference(""), []);
  assert.deepEqual(parseInputSourcesPreference(undefined), []);
  assert.deepEqual(parseInputSourcesPreference("korean,,rime,korean"), ["korean"]);
});

test("what can be added: what the engine has and the person has not added, in the session's order", () => {
  assert.deepEqual(addable(parseInputSources(told)!), ["japanese", "korean"]);
  assert.deepEqual(addable({ ...parseInputSources(told)!, available: ["korean", "pinyin"] }), ["korean"]);
  assert.deepEqual(addable({ ...parseInputSources(told)!, available: [] }), []);
});

test("adding goes at the end, removing takes one; never the layout", () => {
  assert.deepEqual(withAdded(["pinyin"], "japanese"), ["pinyin", "japanese"]);
  assert.deepEqual(withAdded(["pinyin"], "pinyin"), ["pinyin"]);
  assert.deepEqual(withRemoved(["pinyin", "japanese"], "pinyin"), ["japanese"]);
  assert.equal(removable("layout"), false);
  assert.equal(removable(null), false);
  assert.equal(removable("korean"), true);
});

test("the row selected once one is removed: the one in its place, else the one before", () => {
  const sources = ["layout", "pinyin", "japanese"] as const;
  assert.equal(selectedAfterRemoving(sources, "pinyin"), "japanese");
  assert.equal(selectedAfterRemoving(sources, "japanese"), "pinyin");
  assert.equal(selectedAfterRemoving(["layout", "korean"], "korean"), "layout");
  assert.equal(selectedAfterRemoving(sources, "korean"), "layout");
});

test("a layout's name as people read it, in each language; its short name as the top bar's", () => {
  const en = messagesFor("en").keyboard;
  const es = messagesFor("es").keyboard;
  assert.equal(layoutName("es", en.layoutNames), "Spanish");
  assert.equal(layoutName("es", es.layoutNames), "Español");
  assert.equal(layoutName("us", en.layoutNames), "U.S.");
  assert.equal(layoutName("us", es.layoutNames), "EE. UU.");
  // A variant, or a layout the catalogue does not name, as XKB names it.
  assert.equal(layoutName("es (cat)", en.layoutNames), "es (cat)");
  assert.equal(layoutName("ro", en.layoutNames), "ro");
  assert.equal(layoutName("", en.layoutNames), "");
  assert.equal(layoutShortName("es"), "ES");
  assert.equal(layoutShortName("de(neo)"), "DE");
});

test("the input methods are named as macOS names them, in each language", () => {
  assert.deepEqual(messagesFor("en").keyboard.methods, {
    pinyin: "Pinyin – Simplified",
    japanese: "Japanese – Romaji",
    korean: "2-Set Korean",
  });
  assert.deepEqual(messagesFor("es").keyboard.methods, {
    pinyin: "Pinyin – Simplificado",
    japanese: "Japonés – Romaji",
    korean: "Coreano de 2 conjuntos",
  });
  assert.equal(messagesFor("es").keyboard.added("Japonés – Romaji"), "Se ha añadido Japonés – Romaji");
  assert.equal(messagesFor("en").keyboard.removeSource("2-Set Korean"), "Remove 2-Set Korean");
});
