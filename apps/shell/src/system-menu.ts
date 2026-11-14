/**
 * The top bar's Oxy menu, OxyOS's Apple menu (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
 * decision 2): its rows in a Mac's order, their chords as the session shows chords, and the
 * alternates Option (Alt) shows in their items' places while it is held. Pure, so its order, words
 * and chords are tested without drawing it.
 */
import { chordLabel } from "./chord.ts";
import type { ShellMessages } from "./messages";

/** About This Computer's application (an entry never listed, `NoDisplay`: the Oxy menu opens it),
    and the App Store's, Atlas (docs/adr/0030-installing-applications.mdx). */
export const ABOUT_THIS_COMPUTER = "so.oxy.aboutthiscomputer";
export const APP_STORE = "so.oxy.atlas";

/** What a row asks: About This Computer, Settings and the App Store, or the session's ends. */
export type SystemMenuCommand =
  | "system.aboutThisComputer"
  | "system.settings"
  | "system.store"
  | "system.forceQuit"
  | "power.sleep"
  | "power.restart"
  | "power.shutDown"
  | "session.lock"
  | "session.logOut";

export type SystemMenuRow =
  | {
      kind: "item";
      /** Unique in the menu: the command, with `.now` for its alternate. */
      key: string;
      id: SystemMenuCommand;
      label: string;
      /** The chord shown at the row's end; the keymap or the compositor runs it, never the menu. */
      chord?: string;
      /** False for Option's alternates: chosen, the session acts at once, with no question. */
      confirm?: boolean;
      /** The modifiers that show it in place of the item before it, as a Mac's alternate item
          (Bloom's `MenubarItem` `alternate`): Option (`Alt`). */
      alternate?: "Alt";
    }
  | { kind: "separator"; id: string };

/**
 * The Oxy menu's rows, a Mac's Apple menu's in its order: About This Computer; System Settings…
 * and, where the App Store is installed (Atlas), its row named as the registry names it; then,
 * where the platform ends the session (`power`), Force Quit… (Alt+Super+Esc, a Mac's ⌥⌘⎋, ADR 0067),
 * then Sleep, Restart… and Shut Down…; then Lock Screen
 * (Ctrl+Super+Q, a Mac's ⌃⌘Q) where it locks, and Log Out *name*… (Shift+Super+Q, ⇧⌘Q) where it
 * ends the session, always naming the person, as a Mac's does ("Oxy Account" for an account with
 * no name). Each of Restart…, Shut Down… and Log Out *name*… is followed by its alternate, shown in
 * its place while Option is held, as a Mac's: without the ellipsis and the question, and Log Out
 * with Alt+Shift+Super+Q (⌥⇧⌘Q).
 *
 * Launchpad and Mission Control are not in it, as they are not in a Mac's: they are the dock's
 * (its Applications and Show Desktops), their keys' (Super, Super+↑) and the desktop's View menu's.
 * A Mac's Recent Items has no OxyOS equal yet.
 */
export function systemMenuRows(
  messages: ShellMessages,
  {
    canLock,
    power,
    name,
    store,
  }: {
    canLock: boolean;
    /** Whether the platform sleeps, restarts, shuts down and logs out. */
    power: boolean;
    /** The session's user, as the menu names it: their full name, else their user name; none, an
        account with neither (`unnamedAccount`, as the login screen calls it), never the Unix user. */
    name?: string;
    /** The App Store's name as the registry gives it, where it is installed. */
    store?: string;
  },
): SystemMenuRow[] {
  const words = messages.power;
  const person = name?.trim() || words.unnamedAccount;
  const item = (id: SystemMenuCommand, label: string, chord?: string): SystemMenuRow => ({
    kind: "item",
    key: id,
    id,
    label,
    ...(chord ? { chord } : null),
  });
  // Option's alternate of `id`: no question.
  const now = (id: SystemMenuCommand, label: string, chord?: string): SystemMenuRow => ({
    ...item(id, label, chord),
    key: `${id}.now`,
    confirm: false,
    alternate: "Alt",
  });
  const rows: SystemMenuRow[] = [
    item("system.aboutThisComputer", messages.topBar.aboutThisComputer),
    { kind: "separator", id: "system.separator" },
    item("system.settings", messages.topBar.systemSettings),
  ];
  if (store) rows.push(item("system.store", messages.topBar.appStore(store)));
  // Force Quit… (Alt+Super+Esc, a Mac's ⌥⌘⎋), where the session shows it
  // (docs/adr/0067-hangs-and-force-quit.mdx), as the Apple menu's.
  if (power) {
    rows.push(
      { kind: "separator", id: "system.forceQuitSeparator" },
      item("system.forceQuit", messages.topBar.forceQuit, chordLabel(["Alt", "Super", "Esc"], messages)),
    );
  }
  if (power) {
    rows.push(
      { kind: "separator", id: "power.separator" },
      item("power.sleep", words.sleep),
      item("power.restart", words.restartMenu),
      now("power.restart", words.restartNow),
      item("power.shutDown", words.shutDownMenu),
      now("power.shutDown", words.shutDownNow),
    );
  }
  if (canLock || power) rows.push({ kind: "separator", id: "session.separator" });
  if (canLock) {
    rows.push(item("session.lock", messages.topBar.lockScreen, chordLabel(["Ctrl", "Super", "Q"], messages)));
  }
  if (power) {
    rows.push(
      item("session.logOut", words.logOutMenu(person), chordLabel(["Shift", "Super", "Q"], messages)),
      now("session.logOut", words.logOutNow(person), chordLabel(["Alt", "Shift", "Super", "Q"], messages)),
    );
  }
  return rows;
}

/**
 * The rows a menu shows with Option held or not: each alternate in its item's place while it is,
 * the item while it is not, as Bloom's menu draws them. For reading the menu without drawing it.
 */
export function shownRows(rows: readonly SystemMenuRow[], option: boolean): SystemMenuRow[] {
  const shown: SystemMenuRow[] = [];
  for (const row of rows) {
    if (row.kind === "item" && row.alternate) {
      if (option) shown[shown.length - 1] = row;
      continue;
    }
    shown.push(row);
  }
  return shown;
}
