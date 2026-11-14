// `windows` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0016-windows.mdx,
// docs/adr/0021-menu-bar.mdx): the application's own window, its state and operations, its menus in
// the shell's bar, its keys before the focused view, and its first responder for the Edit menu's
// standard actions. Every application, its own windows; the shell has none of its own (its surfaces
// are the session's), but it takes the keys its surfaces receive.
import { followed, type Followed } from './followed.ts';
import type { Events } from './session.ts';

type Json = Record<string, unknown>;

/** The host's modules behind it (OxyOSWindow, OxyOSMenuBar, OxyOSKeyboard, OxyOSEditing). */
export interface WindowModule {
  getConstants(): { available?: boolean; layouts?: boolean; metrics?: Json };
  getState(): Json;
  close(): void;
  minimize(): void;
  zoom(): void;
  startMove(): boolean;
  setSizeLimits(minWidth: number, minHeight: number, maxWidth: number, maxHeight: number): void;
  /** Its zoom control's layouts (docs/adr/0016-windows.mdx, decision 12): a host whose compositor
      lays the window out at its own request (`layouts`), by Bloom's PlatformWindowLayout name. */
  layOut?(layout: string): void;
  moveToDesktop?(desktop: number): void;
}
export interface MenuBarModule {
  getConstants(): { available?: boolean };
  /** The entries as a new commit: its serial, 0 if not sent. */
  setMenus(entries: MenuEntry[]): number;
}
export interface KeyboardModule {
  getConstants(): { available?: boolean };
  /** The one listener the host asks with each key press, waiting for whether it took it. */
  setListener(listener: (event: KeyEvent) => boolean): void;
}
export interface EditingModule {
  getConstants(): { available?: boolean };
  getState(): Json | null;
  perform(action: string): boolean;
}

/** One entry of the window's menus, as the protocol has them (docs/adr/0021-menu-bar.mdx): a menu,
    an item, a submenu or a separator, under its parent's index (-1 for a menu), with its flags
    (disabled 1, checkable 2, checked 4), its label and its shortcut. */
export interface MenuEntry {
  parent: number;
  kind: number;
  flags: number;
  label: string;
  shortcut: string;
}
/** A key the window received, before the focused view. */
/** The modifiers held, as the compositor tells them when they change (protocol version 38, a Mac's
    `flagsChanged:`). */
export interface Modifiers {
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export interface KeyEvent {
  key: string;
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  repeat: boolean;
}

export function createWindows(
  modules: {
    window: WindowModule | null | undefined;
    menuBar: MenuBarModule | null | undefined;
    keyboard: KeyboardModule | null | undefined;
    editing: EditingModule | null | undefined;
  },
  events: Events,
) {
  const { window, menuBar, keyboard, editing } = modules;
  const windowConstants = window?.getConstants();
  const hasWindow = Boolean(window && windowConstants?.available);

  // The window's state: one object per change, so a snapshot changes only when the window does.
  const state = followed<Json>();
  if (hasWindow && window) {
    state.tell(window.getState());
    events.addListener('OxyOSWindowChanged', (next: Json) => state.tell(next));
  }

  // The menus' choices: a commit's serial and an entry's index.
  const choices = new Set<(choice: { serial: number; index: number }) => void>();
  if (menuBar?.getConstants()?.available) {
    events.addListener('OxyOSMenuActivated', (choice: { serial: number; index: number }) => {
      for (const listener of [...choices]) listener(choice);
    });
  }

  // Keys: the host asks one listener, which asks each of the window's in turn until one takes it.
  const keyListeners = new Set<(event: KeyEvent) => boolean | void>();
  const keysAvailable = Boolean(keyboard?.getConstants()?.available);
  const modifierListeners = new Set<(modifiers: Modifiers) => void>();
  if (keysAvailable && keyboard) {
    keyboard.setListener((event) => [...keyListeners].some((listener) => listener(event) === true));
    events.addListener('OxyOSModifiersChanged', (modifiers: Modifiers) => {
      for (const listener of [...modifierListeners]) listener(modifiers);
    });
  }

  // The first responder's state: what the focused text input can do now, null while none has focus.
  const editingAvailable = Boolean(editing?.getConstants()?.available);
  const editingState = followed<Json | null>();
  if (editingAvailable && editing) {
    editingState.tell(editing.getState() ?? null);
    events.addListener('OxyOSEditingChanged', (next: Json | null) => editingState.tell(next ?? null));
  }

  return {
    /** Whether this is an application's window (the shell and an offscreen host have none). */
    available: hasWindow,
    /** Where the platform draws the window's controls and the room they take, and its corner
        radius: the same for every application. */
    metrics: (windowConstants?.metrics ?? null) as Json | null,
    /** The window's state (active, zoomed, full screen, ...), followed. */
    state: state as Followed<Json>,
    close: () => window?.close(),
    minimize: () => window?.minimize(),
    zoom: () => window?.zoom(),
    /** While a press is being handled: the compositor moves the window until its buttons are up.
        Whether it did (no press held, no move). */
    startMove: () => Boolean(window?.startMove()),
    /** The sizes the application's layout holds together at: the compositor never configures the
        window outside them, by its edges or by zoom (0: no limit). */
    setSizeLimits(limits: { minWidth?: number; minHeight?: number; maxWidth?: number; maxHeight?: number }) {
      window?.setSizeLimits(limits.minWidth ?? 0, limits.minHeight ?? 0, limits.maxWidth ?? 0, limits.maxHeight ?? 0);
    },
    /** Whether the compositor lays the window out at its own request (protocol version 50): its
        zoom control's layouts (docs/adr/0016-windows.mdx, decision 12). */
    layouts: Boolean(hasWindow && windowConstants?.layouts && window?.layOut),
    /** Lays the window out as a Mac's green button does (`fill`, `left`, `leftRight`,
        `returnToPrevious`, ...: Bloom's PlatformWindowLayout). */
    layOut: (layout: string) => window?.layOut?.(layout),
    /** Moves the window to another of the session's desktops, from 0 (the state's `desktops`). */
    moveToDesktop: (desktop: number) => window?.moveToDesktop?.(desktop),
    /** The window's menus in the shell's bar. */
    menus: {
      available: Boolean(menuBar?.getConstants()?.available),
      /** Sends `entries` as the window's menus, a new commit: its serial, 0 if not sent (the
          compositor refuses more than 256 entries or deeper than 4). */
      set: (entries: MenuEntry[]): number => menuBar?.setMenus(entries) ?? 0,
      /** The person chose the entry `index` of the commit `serial` in the shell's bar. */
      onChoose(listener: (choice: { serial: number; index: number }) => void): () => void {
        choices.add(listener);
        return () => {
          choices.delete(listener);
        };
      },
    },
    /** The window's keys, offered before the focused view: a listener answering true takes it. */
    keys: {
      available: keysAvailable,
      onKey(listener: (event: KeyEvent) => boolean | void): () => void {
        keyListeners.add(listener);
        return () => {
          keyListeners.delete(listener);
        };
      },
      /** The modifiers held as they change (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
          Option going down or up told at once, with no key or move after it. */
      onModifiers(listener: (modifiers: Modifiers) => void): () => void {
        modifierListeners.add(listener);
        return () => {
          modifierListeners.delete(listener);
        };
      },
    },
    /** The window's first responder for the Edit menu's standard actions (copy, paste, select
        all, ...): what it can do now, followed, and doing one. */
    editing: {
      available: editingAvailable,
      state: editingState as Followed<Json | null>,
      /** Does `action` on the focused text input: whether it did. */
      perform: (action: string): boolean => editing?.perform(action) === true,
    },
  };
}

export type Windows = ReturnType<typeof createWindows>;
