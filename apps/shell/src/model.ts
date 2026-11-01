import type { ImageSourcePropType } from "react-native";

/** Data supplied by the OxyOS application registry and compositor, never invented by the UI. */
export type ShellApplication = {
  id: string;
  name: string;
  icon: ImageSourcePropType | { svg: string };
  pinned?: boolean;
};
export type ShellFolder = {
  id: string;
  name: string;
  appIds: readonly string[];
};
export type ShellWorkspace = { id: string; name: string };
export type ShellWindow = {
  id: string;
  appId: string;
  title: string;
  workspaceId: string;
  thumbnail?: ImageSourcePropType;
  /** Pixel dimensions of the supplied thumbnail, or of the window itself where the platform shows
      it live (ADR 0022); preserves the surface aspect ratio. */
  thumbnailSize?: { width: number; height: number };
  minimized?: boolean;
  /** Hidden with its application (Super+H): off the screen and not in the dock, until the
      application is activated again (docs/adr/0024-system-shortcuts.mdx). */
  hidden?: boolean;
  /** How recently it was the active window: higher is more recent; the application switcher
      orders applications by it. */
  used?: number;
  /** False when the session did not start the window's process: its `appId` is then only its
      own claim, and it is no registry application's (docs/adr/0013-launching-applications.mdx). */
  launched?: boolean;
  /** The active window (the one with the keyboard): its application's dock menu checks it. */
  focused?: boolean;
  /** False while its process does not answer the platform (docs/adr/0034-the-dock.mdx): its dock
      menu says so and offers Force Quit. */
  responding?: boolean;
};
/** An entry of a menu in the top bar: a command (`checked` set when it has a check, `shortcut` its
    chord as shown), a submenu, or a separator. */
export type ShellMenuItem =
  | {
      kind?: "item";
      id: string;
      label: string;
      enabled?: boolean;
      checked?: boolean;
      shortcut?: string;
    }
  | {
      kind: "submenu";
      id: string;
      label: string;
      enabled?: boolean;
      items: readonly ShellMenuItem[];
    }
  | { kind: "separator"; id: string };
/** A menu of the top bar. The first is drawn in bold: the active application's own menu (its
    name, About, Hide, Quit), or the shell's when no application is active
    (docs/adr/0021-menu-bar.mdx). */
export type ShellMenu = {
  label: string;
  items: readonly ShellMenuItem[];
};
export type ShellSnapshot = {
  applications: readonly ShellApplication[];
  folders: readonly ShellFolder[];
  windows: readonly ShellWindow[];
  workspaces: readonly ShellWorkspace[];
  workspaceId: string;
  activeAppId?: string;
  menus: readonly ShellMenu[];
  /** The applications the session opens at login, by id (docs/adr/0034-the-dock.mdx); absent
      where the platform has none to offer. */
  openAtLogin?: readonly string[];
  /** The Trash at the dock's end (docs/adr/0034-the-dock.mdx): how many items it holds, where the
      platform says; absent, the dock has no Trash. */
  trash?: { items?: number };
};
/** Requests go to the platform owner. Rendering a shell never launches a fake app or manages native windows itself. */
export type ShellActions = {
  activateApp: (id: string) => void;
  focusWindow: (id: string) => void;
  /** Gives the keyboard back to the active window, as the top bar driven from the keyboard is
      left (docs/adr/0024-system-shortcuts.mdx); where the platform has no such focus, nothing. */
  returnKeyboard?: () => void;
  closeWindow: (id: string) => void;
  activateWorkspace: (id: string) => void;
  /** Intent only. The owning platform publishes the confirmed assignment in its next snapshot. */
  moveWindowToWorkspace: (windowId: string, workspaceId: string) => void;
  addWorkspace: () => void;
  menuCommand: (id: string) => void;
  /** Hides or quits an application (all its windows), where the platform can: the application
      switcher's H and Q (docs/adr/0024-system-shortcuts.mdx). */
  hideApp?: (id: string) => void;
  quitApp?: (id: string) => void;
  /** The dock's application menu (docs/adr/0034-the-dock.mdx), where the platform can: the
      application ended by the platform at once (it is told nothing and cannot refuse); the
      applications kept in the dock replaced, in order; one opened at login or not. Each is an
      intent: the next snapshot says what the platform kept. */
  forceQuitApp?: (id: string) => void;
  setDock?: (ids: readonly string[]) => void;
  setOpenAtLogin?: (ids: readonly string[]) => void;
  /** The Trash: shown in the file manager, and emptied (the platform asks the person first). */
  openTrash?: () => void;
  emptyTrash?: () => void;
  /** Files dropped on the dock's Trash (`file://` URIs), put in it by the platform, citing the drop
      (docs/adr/0035-drag-and-drop.mdx). */
  trashFiles?: (uris: readonly string[]) => void;
};
/**
 * The session's lock (docs/adr/0010-lock-screen.mdx), owned by the platform: whether it is
 * locked, whom it is locked for, and how the lock screen asks. Only the session unlocks.
 */
export type ShellLock = {
  locked: boolean;
  user: { name: string; fullName?: string };
  /** Whether Caps Lock is on: the password field says so. */
  capsLock?: boolean;
  lockSession: () => void;
  /** Sends the password for the session to check; false if it could not be sent. */
  authenticate: (password: string) => boolean;
  /** The latest answer, a new `id` for each; `wait`: seconds until another may be tried. */
  answer?: {
    id: number;
    outcome: "accepted" | "rejected" | "busy" | "throttled";
    wait: number;
  };
};
/**
 * A notification an application posted (docs/adr/0026-notifications.mdx), as the session keeps it.
 * `appId` is the session's (the posting process's launch record), never the application's claim.
 * `withheld`: the session is locked and the lock screen may not show what it says, so only whose
 * it is was told. `banner`: it arrived to be shown now (the session's preferences allow it).
 */
export type ShellNotification = {
  id: string;
  appId: string;
  title: string;
  subtitle?: string;
  body: string;
  /** Its actions, in order (action n is index n-1); a reply action has a placeholder. */
  actions: readonly { label: string; reply?: string }[];
  /** When it was posted, ms since the epoch. */
  postedAt: number;
  banner?: boolean;
  withheld?: boolean;
};
/**
 * The session's notifications, owned by the platform: the Notification Center's list, the policy
 * the user's preferences set, and what the shell asks. Only the platform reaches the application.
 */
export type ShellNotifications = {
  list: readonly ShellNotification[];
  /** Do Not Disturb: no banner is shown and nothing is said; every one is still kept. */
  doNotDisturb: boolean;
  /** The lock screen may show what notifications say; without it, only how many each has. */
  previews: boolean;
  /** The person pressed it (`action` 0) or its action n, with the reply typed for a reply. */
  activate: (id: string, action: number, reply?: string) => void;
  /** Removes it from the Notification Center. */
  remove: (id: string) => void;
  /** The Control Center's Do Not Disturb: asks the session to change the user's preference; the
      next `doNotDisturb` says what came of it. Without it the Control Center has no switch. */
  setDoNotDisturb?: (enabled: boolean) => void;
};
/** What the platform asks of the shell, a new `id` for each: dismissing what is open, showing the
    launcher or the overview, and the system shortcuts it shows (docs/adr/0024-system-shortcuts.mdx):
    the application switcher's moves, quit, hide, end with the choice (Super released) or cancel,
    and the launcher's search. */
export type ShellCommand = {
  id: number;
  action:
    | "dismiss"
    | "launcher"
    | "overview"
    | "search"
    | "switcher-next"
    | "switcher-previous"
    | "switcher-quit"
    | "switcher-hide"
    | "switcher-done"
    | "switcher-cancel"
    // Super+F2: the keyboard to the top bar's menus, as a Mac's Control-F2.
    | "menu-bar";
};

/** Whether `window` is the application `appId`'s: its id, given by the session's launch record
    or (when the platform says nothing of launch records) its own. A window the session did not
    start is no registry application's, whatever it claims. */
export function belongsTo(window: ShellWindow, appId: string): boolean {
  return window.launched !== false && window.appId === appId;
}

/** A level the session controls (docs/adr/0025-hardware-keys-and-control-center.mdx): 0 to 1,
    a volume's mute, and whether the machine has it at all. */
export type ShellLevel = { level: number; muted?: boolean; available: boolean };
export type ShellControl = "output" | "input" | "display" | "keyboard";
export type ShellBattery = {
  battery: "absent" | "discharging" | "charging" | "full" | "not-charging";
  /** 0 to 100. */
  percent: number;
  onAC: boolean;
  /** Until empty (discharging) or full (charging); 0 unknown. */
  minutes: number;
};
export type ShellNetwork = {
  wifi: "unavailable" | "off" | "disconnected" | "connecting" | "connected";
  /** The access point's signal, 0 to 100, and its name (empty unless connected or connecting). */
  signal: number;
  ssid: string;
  wired: boolean;
};
export type ShellBluetooth = { state: "unavailable" | "off" | "on"; connected: number };
/**
 * The machine as the session says it is (docs/adr/0025-hardware-keys-and-control-center.mdx),
 * never invented by the UI: what it has not said is not shown. The Control Center's changes are
 * intents; the next state says what happened.
 */
export type ShellControls = {
  levels: Partial<Record<ShellControl, ShellLevel>>;
  power?: ShellBattery;
  network?: ShellNetwork;
  bluetooth?: ShellBluetooth;
  /** The latest level a hardware key acted on, a new `id` for each: the bezel shows it. */
  bezel?: { id: number; control: ShellControl };
  setLevel?: (control: ShellControl, level: number) => void;
  setMuted?: (control: ShellControl, muted: boolean) => void;
  setWifi?: (enabled: boolean) => void;
  /** The appearance, as a preference the session keeps (ADR 0019). */
  setDark?: (dark: boolean) => void;
  /** The Control Center opened: the session looks at the machine again. */
  refresh?: () => void;
};

/** A screenshot the session saved (docs/adr/0024-system-shortcuts.mdx, decision 10): the session's
    number for it, the display it is of, its pixels and its file. */
export type ShellScreenshot = { id: number; output: number; width: number; height: number; path: string };
/**
 * Screenshots as the platform takes them (docs/adr/0024-system-shortcuts.mdx, decision 10): the
 * compositor opens the interface from the keyboard (Super+Shift+4, Super+Shift+5), drives its keys
 * (Space, Return, Escape) and captures; the shell shows the interface and the thumbnail and says
 * what the person picked. Rectangles are the global space's logical pixels.
 */
export type ShellScreenshots = {
  /** This shell's display: its id, where it is in the global space and its scale. */
  output: { id: number; x: number; y: number; scale: number };
  /** The interface open now (`serial` new for each), and how many times Return was pressed in its
      toolbar; null while closed. */
  active: { serial: number; mode: "selection" | "window" | "toolbar"; output: number; confirm: number } | null;
  /** The window under the pointer while the interface is open (its rectangle without shadow). */
  target: { window: number; x: number; y: number; width: number; height: number } | null;
  /** The screenshots saved, in order (each of one display). */
  saved: readonly ShellScreenshot[];
  /** Captures what the person picked: every display, a rectangle of `output`, or a window. */
  capture: (
    kind: "screens" | "area" | "window",
    output: number,
    rect: { x: number; y: number; width: number; height: number } | null,
    window: number,
    destination: "desktop" | "documents",
  ) => void;
  /** Ends the interface without a capture (the toolbar's close). */
  end: () => void;
  /** Opens a saved screenshot in the application that opens it. */
  open: (id: number) => void;
};
