/**
 * The top bar's Oxy menu, OxyOS's Apple menu (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
 * decision 2): its rows in a Mac's order, their chords as the session shows chords, and the
 * alternates Option (Alt) shows while it is held. Pure, so its order, words and chords are tested
 * without drawing it.
 */
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
  | "power.sleep"
  | "power.restart"
  | "power.shutDown"
  | "session.lock"
  | "session.logOut";

export type SystemMenuRow =
  | {
      kind: "item";
      id: SystemMenuCommand;
      label: string;
      /** The chord shown at the row's end; the keymap or the compositor runs it, never the menu. */
      chord?: string;
      /** False for Option's alternates: chosen, the session acts at once, with no question. */
      confirm?: boolean;
    }
  | { kind: "separator"; id: string };

/** A chord as the shell's menus show one: its keys joined by "+", Shift in the catalogue's word
    (Mayús in Spanish), Ctrl, Alt and Super as the keys are labelled. */
export function chordLabel(keys: readonly string[], messages: ShellMessages): string {
  return keys.map((key) => (key === "Shift" ? messages.menus.shift : key)).join("+");
}

/**
 * The Oxy menu's rows, a Mac's Apple menu's in its order: About This Computer; System Settings…
 * and, where the App Store is installed (Atlas), its row named as the registry names it; then,
 * where the platform ends the session (`power`), Sleep, Restart… and Shut Down…; then Lock Screen
 * (Ctrl+Super+Q, a Mac's ⌃⌘Q) where it locks, and Log Out *name*… (Shift+Super+Q, ⇧⌘Q) where it
 * ends the session, always naming the person, as a Mac's does ("Oxy Account" for an account with
 * no name). With `option` held, as a Mac's alternates: Restart, Shut Down and Log Out *name* lose
 * their ellipsis and their question, and Log Out shows Alt+Shift+Super+Q (⌥⇧⌘Q).
 *
 * Launchpad and Mission Control are not in it, as they are not in a Mac's: they are the dock's
 * (its Applications and Show Desktops), their keys' (Super, Super+↑) and the desktop's View menu's.
 * A Mac's Recent Items has no OxyOS equal yet, and Force Quit… comes with its window.
 */
export function systemMenuRows(
  messages: ShellMessages,
  {
    canLock,
    power,
    name,
    store,
    option = false,
  }: {
    canLock: boolean;
    /** Whether the platform sleeps, restarts, shuts down and logs out. */
    power: boolean;
    /** The session's user, as the menu names it: their full name, else their user name; none, an
        account with neither (`unnamedAccount`, as the login screen calls it), never the Unix user. */
    name?: string;
    /** The App Store's name as the registry gives it, where it is installed. */
    store?: string;
    /** Whether Option (Alt) is held while the menu is open. */
    option?: boolean;
  },
): SystemMenuRow[] {
  const words = messages.power;
  const person = name?.trim() || words.unnamedAccount;
  const rows: SystemMenuRow[] = [
    { kind: "item", id: "system.aboutThisComputer", label: messages.topBar.aboutThisComputer },
    { kind: "separator", id: "system.separator" },
    { kind: "item", id: "system.settings", label: messages.topBar.systemSettings },
  ];
  if (store) rows.push({ kind: "item", id: "system.store", label: messages.topBar.appStore(store) });
  if (power) {
    rows.push(
      { kind: "separator", id: "power.separator" },
      { kind: "item", id: "power.sleep", label: words.sleep },
      {
        kind: "item",
        id: "power.restart",
        label: option ? words.restartNow : words.restartMenu,
        confirm: !option,
      },
      {
        kind: "item",
        id: "power.shutDown",
        label: option ? words.shutDownNow : words.shutDownMenu,
        confirm: !option,
      },
    );
  }
  if (canLock || power) rows.push({ kind: "separator", id: "session.separator" });
  if (canLock) {
    rows.push({
      kind: "item",
      id: "session.lock",
      label: messages.topBar.lockScreen,
      chord: chordLabel(["Ctrl", "Super", "Q"], messages),
    });
  }
  if (power) {
    rows.push({
      kind: "item",
      id: "session.logOut",
      label: option ? words.logOutNow(person) : words.logOutMenu(person),
      chord: chordLabel(option ? ["Alt", "Shift", "Super", "Q"] : ["Shift", "Super", "Q"], messages),
      confirm: !option,
    });
  }
  return rows;
}
