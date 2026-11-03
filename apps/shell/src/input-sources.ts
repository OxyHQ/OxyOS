/**
 * Input sources and the candidate window (docs/adr/0041-input-methods.mdx, decisions 5 and 6), as
 * the session tells them: the person's sources (the keyboard layout first, then the input methods
 * they added), the one in effect, and the page of candidates an input method hands the system to
 * show at the caret. What the top bar and the candidate window make of them; nothing here asks
 * anything.
 */

import type { ShellMessages } from "./messages";

/** The input sources as the session tells them (`{"inputSources": …}`). */
export type InputSourcesState = {
  /** The session's keyboard layout, by its XKB name (`es`, `us`). */
  layout: string;
  /** The source in effect: "layout" or an input method's id. */
  current: string;
  /** The person's sources, the layout first ("layout"). */
  sources: readonly string[];
  /** The input methods the session's engine has. */
  available: readonly string[];
};

/** One page of candidates to show at the caret, in the input method's order. */
export type CandidateList = {
  candidates: readonly string[];
  /** A reading (pinyin's "ni'hao"), a mode; empty for none. */
  aux: string;
  /** The highlighted one's index; -1 for none. */
  highlighted: number;
  /** A page before this one, a page after it. */
  previous: boolean;
  next: boolean;
  /** Laid out down, one under the other. */
  vertical: boolean;
};

/** The top bar's mark for an input method, as a Mac's Input menu shows it. */
const SHORT: Record<string, string> = { pinyin: "拼", japanese: "あ", korean: "한" };

/** What the top bar shows for a source: the layout's code uppercased (`ES`, `US`), or the input
    method's character (拼, あ, 한). */
export function sourceMark(id: string, state: Pick<InputSourcesState, "layout">): string {
  if (id === "layout") return state.layout.split(/[(+:]/)[0].toUpperCase();
  return SHORT[id] ?? id.slice(0, 2).toUpperCase();
}

/** A source's name, in the session's language: the layout's ("Spanish", "Español"), or the input
    method's ("Pinyin – Simplified", "Pinyin – Simplificado"). */
export function sourceName(id: string, state: Pick<InputSourcesState, "layout">, messages: ShellMessages): string {
  const words = messages.inputSources;
  if (id === "layout") {
    const code = state.layout.split(/[(+:]/)[0];
    return words.layouts[code] ?? words.layout(code.toUpperCase());
  }
  if (id === "pinyin" || id === "japanese" || id === "korean") return words[id];
  return id;
}

/** Whether the top bar shows the item: only with more than one source, as a Mac's Input menu. */
export function showsInputSources(state: InputSourcesState | null | undefined): state is InputSourcesState {
  return !!state && state.sources.length > 1;
}

/** A candidate's number on the page, as the digit keys choose it: 1 to 9, then 0. */
export function candidateNumber(index: number): string {
  return String((index + 1) % 10);
}

/** A candidate as the screen reader says it, its place and the page's count: "你好, 1 of 9". */
export function candidateLabel(list: CandidateList, index: number, messages: ShellMessages): string {
  return messages.candidates.candidate(list.candidates[index] ?? "", index + 1, list.candidates.length);
}

/** What the candidate list says of itself: the highlighted candidate with its place, which it
    says again as the highlight or the page changes (a polite live region); "Candidates" while none
    is highlighted. */
export function candidatesSaid(list: CandidateList, messages: ShellMessages): string {
  if (list.highlighted < 0 || list.highlighted >= list.candidates.length) return messages.candidates.list;
  return candidateLabel(list, list.highlighted, messages);
}
