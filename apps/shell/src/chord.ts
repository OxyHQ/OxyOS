/**
 * A key chord as every menu of the shell shows it (docs/adr/0021-menu-bar.mdx, "One notation"):
 * the application's menus, its own menu, the Oxy menu and the desktop's. One formatter, so a chord
 * reads the same wherever it is shown.
 *
 * - **Key names, joined by "+",** as a PC keyboard labels its keys and as Windows, ChromeOS and
 *   GNOME write chords: Ctrl+Mayús+Z. A Mac writes ⌃⌥⇧⌘ because its keys carry those symbols; a
 *   PC's carry their names, so OxyOS writes the names.
 * - **One order,** a Mac's ⌃⌥⇧⌘ with Super in ⌘'s place: Ctrl, Alt, Shift, Super, then the key.
 *   Ctrl first is also how every PC writes it (Ctrl+Alt+Supr).
 * - **In the session's language:** each name as that language's keyboards label it (Shift is
 *   Mayús, Backspace Retroceso, Delete Supr in Spanish); letters in capitals, arrows as arrows.
 *
 * A chord is read in Bloom's spelling (`Mod+Shift+Z`: `Mod` the applications' command modifier,
 * Ctrl on OxyOS; `Meta` the Super key; `Plus` and `Space` for those keys; W3C key names) or as its
 * keys (`["Ctrl", "Super", "Q"]`, the system's). Pure, so its words are tested without drawing.
 */
import type { ShellMessages } from "./messages";

type Modifier = "ctrl" | "alt" | "shift" | "super";

/** The modifiers in the order they are written. */
const ORDER: readonly Modifier[] = ["ctrl", "alt", "shift", "super"];

/** Each spelling of a modifier, Bloom's and the system's, in lower case. */
const MODIFIERS: Readonly<Record<string, Modifier>> = {
  mod: "ctrl",
  ctrl: "ctrl",
  control: "ctrl",
  alt: "alt",
  option: "alt",
  shift: "shift",
  meta: "super",
  super: "super",
};

/** The keys a chord names that are not characters, as the catalogue names them. */
const NAMED: Readonly<Record<string, keyof ShellMessages["keys"]>> = {
  enter: "enter",
  return: "enter",
  escape: "escape",
  esc: "escape",
  tab: "tab",
  space: "space",
  backspace: "backspace",
  delete: "delete",
  insert: "insert",
  home: "home",
  end: "end",
  pageup: "pageUp",
  pagedown: "pageDown",
};

const ARROWS: Readonly<Record<string, string>> = {
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

/** A chord's parts: its modifiers and its key, the last; `Mod++` is `+` held with `Mod`. */
function parts(chord: string | readonly string[]): string[] {
  if (typeof chord !== "string") return [...chord];
  if (chord === "+") return ["+"];
  if (chord.endsWith("++")) return [...chord.slice(0, -2).split("+"), "+"];
  return chord.split("+");
}

/** How a key is written: a letter in capitals, an arrow as an arrow, a named key in the language. */
function keyLabel(key: string, messages: ShellMessages): string {
  const lower = key.toLowerCase();
  if (lower === "plus") return "+";
  const named = NAMED[lower];
  if (named) return messages.keys[named];
  const arrow = ARROWS[lower];
  if (arrow) return arrow;
  return key.length === 1 ? key.toUpperCase() : key;
}

/** The chord as the shell's menus show it: `Mod+Shift+Z` is "Ctrl+Shift+Z" ("Ctrl+Mayús+Z" in
    Spanish), `["Shift", "Super", "Q"]` "Shift+Super+Q". Empty for none. */
export function chordLabel(chord: string | readonly string[], messages: ShellMessages): string {
  const all = parts(chord).map((part) => part.trim());
  const key = all.pop();
  if (!key) return "";
  const held = new Set<Modifier>();
  for (const part of all) {
    const modifier = MODIFIERS[part.toLowerCase()];
    if (modifier) held.add(modifier);
  }
  const words = messages.keys;
  return [...ORDER.filter((modifier) => held.has(modifier)).map((modifier) => words[modifier]), keyLabel(key, messages)].join(
    "+",
  );
}
