import type { ShellKeyEvent } from "./platform";

/**
 * What a key does to the top bar while it has the keyboard (Super+F2, as a Mac's Control-F2;
 * docs/adr/0024-system-shortcuts.mdx), given whether one of its menus is open:
 * - Left and Right move to the previous and the next menu, round the ends, the next one opening
 *   when one is open;
 * - with none open, Down, Return and Space open the one with the keyboard, and Escape leaves the
 *   bar, the keyboard back with the window;
 * - with one open, everything else is the menu's own (Up, Down, Home, End, Return, Escape, a
 *   letter: Bloom's popup navigation), so null.
 * A key with Ctrl, Alt or Super is never the bar's.
 */
export function barKey(event: ShellKeyEvent, menuOpen: boolean): "previous" | "next" | "open" | "leave" | null {
  if (event.ctrlKey || event.altKey || event.metaKey) return null;
  if (event.key === "ArrowLeft") return "previous";
  if (event.key === "ArrowRight") return "next";
  if (menuOpen) return null;
  if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") return "open";
  if (event.key === "Escape") return "leave";
  return null;
}
