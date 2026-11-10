import { test } from "node:test";
import assert from "node:assert/strict";
import {
  candidateLabel,
  candidateNumber,
  candidatesSaid,
  showsInputSources,
  sourceMark,
  sourceName,
  switchedSource,
  type CandidateList,
  type InputSourcesState,
} from "../src/input-sources.ts";
import { shellMessages } from "../src/messages.ts";

const ES = shellMessages("es-ES");
const EN = shellMessages("en-US");

const state = (more: Partial<InputSourcesState> = {}): InputSourcesState => ({
  layout: "es",
  current: "layout",
  sources: ["layout", "pinyin", "japanese", "korean"],
  available: ["pinyin", "japanese", "korean"],
  ...more,
});

const list = (more: Partial<CandidateList> = {}): CandidateList => ({
  candidates: ["你好", "拟好", "你号", "倪浩", "泥", "你", "尼", "妮", "霓", "呢"],
  aux: "nihao",
  highlighted: 0,
  previous: false,
  next: true,
  vertical: false,
  ...more,
});

test("the top bar shows the layout's code and each input method's character", () => {
  assert.equal(sourceMark("layout", state()), "ES");
  assert.equal(sourceMark("layout", state({ layout: "us" })), "US");
  // A layout with a variant is still its layout's code.
  assert.equal(sourceMark("layout", state({ layout: "us(intl)" })), "US");
  assert.equal(sourceMark("pinyin", state()), "拼");
  assert.equal(sourceMark("japanese", state()), "あ");
  assert.equal(sourceMark("korean", state()), "한");
});

test("each source is named in the session's language", () => {
  assert.equal(sourceName("pinyin", state(), EN), "Pinyin – Simplified");
  assert.equal(sourceName("pinyin", state(), ES), "Pinyin – Simplificado");
  assert.equal(sourceName("japanese", state(), EN), "Japanese – Romaji");
  assert.equal(sourceName("japanese", state(), ES), "Japonés – Romaji");
  assert.equal(sourceName("korean", state(), EN), "2-Set Korean");
  assert.equal(sourceName("korean", state(), ES), "Coreano de 2 conjuntos");
  assert.equal(sourceName("layout", state(), EN), "Spanish");
  assert.equal(sourceName("layout", state(), ES), "Español");
  assert.equal(sourceName("layout", state({ layout: "us" }), ES), "EE. UU.");
  // A layout it has no name for is the keyboard of its code.
  assert.equal(sourceName("layout", state({ layout: "ch" }), EN), "Keyboard (CH)");
  assert.equal(sourceName("layout", state({ layout: "ch" }), ES), "Teclado (CH)");
});

test("the item shows only with more than one source", () => {
  assert.equal(showsInputSources(null), false);
  assert.equal(showsInputSources(state({ sources: ["layout"] })), false);
  assert.equal(showsInputSources(state({ sources: ["layout", "pinyin"] })), true);
});

test("candidates are numbered 1 to 9, then 0, as the digit keys choose them", () => {
  assert.deepEqual(
    Array.from({ length: 10 }, (_, index) => candidateNumber(index)),
    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
  );
});

test("the list says its highlighted candidate with its place and the count", () => {
  assert.equal(candidatesSaid(list(), ES), "你好, 1 de 10");
  assert.equal(candidatesSaid(list(), EN), "你好, 1 of 10");
  assert.equal(candidatesSaid(list({ highlighted: 3 }), ES), "倪浩, 4 de 10");
  assert.equal(candidateLabel(list(), 9, EN), "呢, 10 of 10");
  // Nothing highlighted: the list is named by what it is.
  assert.equal(candidatesSaid(list({ highlighted: -1 }), ES), "Candidatos");
  assert.equal(candidatesSaid(list({ highlighted: -1 }), EN), "Candidates");
});

test("a switch of the source in use is said; the first state and a list change are not", () => {
  const pinyin = state({ current: "pinyin" });
  // Super+Alt+Space, or the top bar's menu: the list as it was.
  assert.equal(switchedSource(pinyin, state({ current: "japanese" })), "japanese");
  assert.equal(switchedSource(pinyin, state({ current: "layout" })), "layout");
  // The first state told, and a state told again unchanged.
  assert.equal(switchedSource(null, pinyin), null);
  assert.equal(switchedSource(pinyin, state({ current: "pinyin" })), null);
  // The source in use removed in Settings: the session moves to the layout, and Settings says the
  // removal itself.
  assert.equal(switchedSource(pinyin, state({ current: "layout", sources: ["layout", "japanese", "korean"] })), null);
  // A source added while another is in use is no switch either.
  assert.equal(switchedSource(pinyin, state({ current: "pinyin", sources: ["layout", "pinyin"] })), null);
});
