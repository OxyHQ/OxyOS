/**
 * The top bar's Oxy menu, OxyOS's Apple menu (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
 * decision 2): its rows in a Mac's order, their chords as the session shows chords, and the
 * alternates Option (Alt) shows while it is held. Pure, so its order, words and chords are tested
 * without drawing it.
 */
import { chordLabel } from "./chord.ts";
import type { ShellMessages } from "./messages";

/** What a row asks: the shell's own views and Settings, or the session's ends. */
export type SystemMenuCommand =
  | "system.settings"
  | "view.launcher"
  | "view.overview"
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

/**
 * The Oxy menu's rows: System Settings, Applications and Show Desktops; then, where the platform
 * ends the session (`power`), Sleep, Restart… and Shut Down…; then Lock Screen (Ctrl+Super+Q, a
 * Mac's ⌃⌘Q) where it locks, and Log Out *name*… (Shift+Super+Q, ⇧⌘Q) where it ends the session,
 * always naming the person, as a Mac's does ("Oxy Account" for an account with no name).
 * With `option` held, as a Mac's alternates: Restart, Shut Down and Log Out *name* lose their
 * ellipsis and their question, and Log Out shows Alt+Shift+Super+Q (⌥⇧⌘Q).
 */
export function systemMenuRows(
  messages: ShellMessages,
  {
    canLock,
    power,
    name,
    option = false,
  }: {
    canLock: boolean;
    /** Whether the platform sleeps, restarts, shuts down and logs out. */
    power: boolean;
    /** The session's user, as the menu names it: their full name, else their user name; none, an
        account with neither (`unnamedAccount`, as the login screen calls it), never the Unix user. */
    name?: string;
    /** Whether Option (Alt) is held while the menu is open. */
    option?: boolean;
  },
): SystemMenuRow[] {
  const words = messages.power;
  const person = name?.trim() || words.unnamedAccount;
  const rows: SystemMenuRow[] = [
    { kind: "item", id: "system.settings", label: messages.common.systemSettings },
    { kind: "item", id: "view.launcher", label: messages.topBar.applications },
    { kind: "item", id: "view.overview", label: messages.common.showDesktops },
  ];
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
