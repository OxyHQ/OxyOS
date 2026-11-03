/**
 * Input sources (docs/adr/0041-input-methods.mdx, decision 6), as a Mac's: the person's keyboard
 * layout, always first and always there, then the input methods they added, in order. The session
 * keeps them as preferences (`input_sources`, the added ones; `input_source`, the current) and
 * tells Settings what is in effect and what its engine has (`inputSources`, unasked and when
 * asked). Settings changes the list through the preference alone; the session's language never
 * changes it (ADR 0040).
 */

/** The input methods OxyOS has, by the names the preferences keep (`oxyos-preferences`'s
    `INPUT_METHODS`), in the order the session offers them. */
export const INPUT_METHODS = ["pinyin", "japanese", "korean"] as const;
export type InputMethod = (typeof INPUT_METHODS)[number];

/** What the preferences keep for the keyboard layout. */
export const LAYOUT_SOURCE = "layout";
export type InputSourceId = typeof LAYOUT_SOURCE | InputMethod;

/** The input sources as the session tells them. */
export type InputSourcesState = {
  /** The session's keyboard layout, as XKB names it (`es`, `us`, `de(neo)`). */
  layout: string;
  /** The source typing now. */
  current: InputSourceId;
  /** The sources in effect, the layout first. */
  sources: InputSourceId[];
  /** The input methods the session's engine has: what can be added (none when not installed). */
  available: InputMethod[];
};

export function isInputMethod(id: unknown): id is InputMethod {
  return typeof id === "string" && (INPUT_METHODS as readonly string[]).includes(id);
}

/**
 * The state from the session's `inputSources` object, read strictly: each list's members known and
 * none twice, the layout first and only first, the current among the sources. Anything else is
 * not the session's state, and null.
 */
export function parseInputSources(told: unknown): InputSourcesState | null {
  if (typeof told !== "object" || told === null) return null;
  const { layout, current, sources, available } = told as Record<string, unknown>;
  if (typeof layout !== "string" || !Array.isArray(sources) || !Array.isArray(available)) return null;
  if (sources[0] !== LAYOUT_SOURCE) return null;
  const methods = sources.slice(1);
  const distinct = (list: unknown[]) => new Set(list).size === list.length;
  if (!methods.every(isInputMethod) || !distinct(methods)) return null;
  if (!available.every(isInputMethod) || !distinct(available)) return null;
  if (typeof current !== "string" || !(sources as unknown[]).includes(current)) return null;
  return {
    layout,
    current: current as InputSourceId,
    sources: [LAYOUT_SOURCE, ...methods],
    available: [...available],
  };
}

/** The state Settings shows without a session telling it: the preferences' list after the layout,
    with the methods `available` (the browser preview's, or none). */
export function inputSourcesOf(
  layout: string,
  kept: readonly string[],
  current: string,
  available: readonly InputMethod[],
): InputSourcesState {
  const methods = kept.filter(isInputMethod).filter((id, index, all) => all.indexOf(id) === index);
  return {
    layout,
    current: isInputMethod(current) && methods.includes(current) ? current : LAYOUT_SOURCE,
    sources: [LAYOUT_SOURCE, ...methods],
    available: [...available],
  };
}

/** The preference's list read from its text (`pinyin,japanese`): the known names, each once. */
export function parseInputSourcesPreference(value: string | undefined): InputMethod[] {
  const methods = (value ?? "").split(",").filter(isInputMethod);
  return methods.filter((id, index) => methods.indexOf(id) === index);
}

/** The input methods that can be added: those the engine has and the person has not added. */
export function addable(state: InputSourcesState): InputMethod[] {
  return INPUT_METHODS.filter((id) => state.available.includes(id) && !state.sources.includes(id));
}

/** Whether a source can be removed: every one but the layout. */
export function removable(id: InputSourceId | null): id is InputMethod {
  return id !== null && isInputMethod(id);
}

/** The preference's new list with `id` added at its end, as a Mac adds a source below the others. */
export function withAdded(kept: readonly InputMethod[], id: InputMethod): InputMethod[] {
  return kept.includes(id) ? [...kept] : [...kept, id];
}

/** The preference's new list without `id`. */
export function withRemoved(kept: readonly InputMethod[], id: InputMethod): InputMethod[] {
  return kept.filter((kept) => kept !== id);
}

/** The row selected once `removed` is gone from `sources`: the one that took its place, or the
    one before it when it was the last. */
export function selectedAfterRemoving(sources: readonly InputSourceId[], removed: InputSourceId): InputSourceId {
  const at = sources.indexOf(removed);
  const left = sources.filter((id) => id !== removed);
  if (at < 0) return left[0] ?? LAYOUT_SOURCE;
  return left[Math.min(at, left.length - 1)] ?? LAYOUT_SOURCE;
}

/** The base XKB layout of a layout's name (`es` of `es(cat)` or `es (cat)`). */
function baseLayout(layout: string): { base: string; variant: string } {
  const match = /^([a-z]+)\s*(?:\(([^)]*)\))?$/i.exec(layout.trim());
  return match ? { base: match[1].toLowerCase(), variant: match[2] ?? "" } : { base: layout.trim(), variant: "" };
}

/** A layout's short name, as the top bar shows it (`ES`, `US`). */
export function layoutShortName(layout: string): string {
  return baseLayout(layout).base.toUpperCase();
}

/** A layout's name as people read it, from the catalogue's layout names ("Spanish", "U.S."); a
    layout with a variant, or one the catalogue does not name, as XKB names it. */
export function layoutName(layout: string, names: Readonly<Record<string, string>>): string {
  const { base, variant } = baseLayout(layout);
  const named = variant === "" ? names[base] : undefined;
  return named ?? (layout.trim() === "" ? "" : layout.trim());
}

/** What each input method is drawn with in the list, as the top bar shows it (ADR 0041). */
export const INPUT_METHOD_GLYPHS: Record<InputMethod, string> = { pinyin: "拼", japanese: "あ", korean: "한" };
