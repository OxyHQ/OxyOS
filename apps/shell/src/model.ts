import type { AutofillList } from "./autofill";
import type { CandidateList, InputSourcesState } from "./input-sources";
import type { ImageSourcePropType } from "react-native";
import type { NetworkState } from "./network";
import type { BluetoothState } from "./bluetooth";
import type { SoundState } from "./sound";

/** Data supplied by the OxyOS application registry and compositor, never invented by the UI. */
/** The wallpaper (docs/adr/0048-wallpapers.mdx). */
export type { ShellWallpaper, WallpaperFit } from "./wallpaper";

export type ShellApplication = {
  id: string;
  name: string;
  icon: ImageSourcePropType | { svg: string };
  /** When its picture is its own shape (a rounded square, transparent around it and opaque
      within: Dropsea's), the share of the picture its body takes, which the platform measured from
      the picture: drawn as it is, its body the icon grid's. Absent for any other picture, which is
      put in the grid's body on white. */
  iconBody?: number;
  pinned?: boolean;
  /** The media types it opens (its entry's MimeType, docs/adr/0028-file-access.mdx): files of
      them dropped on its dock icon open in it (docs/adr/0034-the-dock.mdx, decision 11). */
  opens?: readonly string[];
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
  /** Minimizes a window into its application's dock icon (the genie, docs/adr/0043-the-genie.mdx),
      where the platform can: the dock's press on the frontmost application (docs/adr/0034-the-dock.mdx,
      decision 9). */
  minimizeWindow?: (id: string) => void;
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
  /** Files dropped on an application's dock icon (`file://` URIs), opened in it by the platform,
      citing the drop (docs/adr/0034-the-dock.mdx, decision 11); and the desktop's files opened
      (docs/adr/0047-finder-drags-and-the-desktop.mdx), citing the press or key that opened them. */
  openFiles?: (id: string, uris: readonly string[]) => void;
  /** A URL opened in the application that opens its scheme, citing the latest input: a desktop
      folder opened in Dropsea (`dropsea://files?uri=…`, docs/adr/0047-finder-drags-and-the-desktop.mdx). */
  openUrl?: (url: string) => void;
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
  /** The account's PIN for this computer (docs/adr/0020-oxy-account-login.mdx, decision 2):
      `set`, or `locked` after too many wrong ones; none without one, or when it cannot be read. */
  pin?: "set" | "locked";
  /** Whether the account's password signs in on this computer; false: the PIN is its one local
      credential. True when it cannot be read. */
  password?: boolean;
  /** Sends the PIN for the session to check as one; false if it could not be sent. */
  authenticatePin?: (pin: string) => boolean;
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
  /** What a system shortcut asks of what it shows (docs/adr/0046-interactive-gestures.mdx): a key's
      toggles it (none says so too); a touchpad's swipe opens Mission Control or App Exposé as its
      fingers begin to drive it, and closes it when they let it go back. */
  intent?: "toggle" | "open" | "close";
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
    | "menu-bar"
    // Super+F3 and Super+F8: the keyboard to the dock and to the status items, as a Mac's
    // Control-F3 and Control-F8 (docs/adr/0039-keyboard-access.mdx).
    | "dock"
    | "status-menus"
    // Super+N: the Notification Center, as a Mac's Fn-N (docs/adr/0039-keyboard-access.mdx).
    | "notifications"
    // Super+↑ and Super+↓, or three or four fingers swiped up and down on the touchpad: Mission
    // Control and App Exposé, as a Mac's Control-↑ and Control-↓ (docs/adr/0045-app-expose.mdx).
    | "mission-control"
    | "app-expose"
    // Super+Space: Spotlight, as a Mac's ⌘Space; again, it closes. Super+Return while it is open
    // shows its selected result in Dropsea, as a Mac's ⌘Return reveals it in Finder.
    | "spotlight"
    | "reveal";
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
/** An application's energy, as the session sampled it as the battery's menu opened
    (docs/adr/0057-battery-and-energy.mdx). */
export type ShellEnergyApp = { id: string; name: string; significant: boolean };
/** The battery's menu (docs/adr/0057-battery-and-energy.mdx): what the session says beyond the
    battery itself, and what the menu asks of it. */
export type ShellBatteryMenu = {
  /** The charge shown beside the icon (the person's `battery_percentage`). */
  showPercentage: boolean;
  /** Low Power Mode is on: the icon's charge is drawn yellow, as a Mac's. */
  lowPower: boolean;
  /** Each application's energy, sampled as the menu opens. */
  energy: () => Promise<readonly ShellEnergyApp[]>;
  /** Battery Settings…: Settings at its Battery page. */
  openSettings: () => void;
};
/** A backup destination as the backup service says it is (docs/adr/0059-backups.mdx): times in
    seconds since the epoch, sizes in bytes. */
export type ShellBackupDestination = {
  id: string;
  name: string;
  kind: "disk" | "share";
  /** A share's `smb://` URL; none for a disk. */
  url: string | null;
  /** Its disk connected (a share: worth trying). */
  available: boolean;
  lastBackup: number | null;
  oldestBackup: number | null;
  backups: number;
  usedBytes: number | null;
  freeBytes: number | null;
  /** Its latest backup that failed, and why (`full`, `unreachable`, `shareRefused`,
      `wrongPassword`, `inUse`, `noBackup`, `damaged`, `failed`). */
  lastFailure: { reason: string; at: number } | null;
};
/** The backup running now. */
export type ShellBackupRunning = {
  /** The destination's id. */
  destination: string;
  phase: "preparing" | "backingUp" | "cleaningUp" | "checking";
  /** Asked for (Back Up Now) rather than scheduled. */
  asked: boolean;
  /** 0 to 1, a fraction. */
  percent: number;
  bytesDone: number;
  bytesTotal: number;
  filesDone: number;
  filesTotal: number;
  secondsRemaining: number | null;
};
/** Backups as the session tells the shell (docs/adr/0059-backups.mdx), unasked as they change. */
export type ShellBackupState = {
  automatic: boolean;
  destinations: ShellBackupDestination[];
  excluded: string[];
  running: ShellBackupRunning | null;
  /** The newest backup of any destination. */
  lastBackup: number | null;
  nextBackup: number | null;
  /** Scheduled backups are held back (Low Power Mode). */
  held: boolean;
  administrator: boolean;
  /** The person's `backup_in_menu_bar` preference. */
  inMenuBar: boolean;
};
/** The menu bar's backup item (docs/adr/0059-backups.mdx, decision 10): the state and what its
    menu asks. */
export type ShellBackupMenu = {
  state: ShellBackupState;
  /** Back Up Now (the session asks the service, citing the press). */
  backUpNow: () => void;
  /** Skip This Backup. */
  skip: () => void;
  /** Browse Backups: opens Dropsea's (dropsea://backups). */
  browse: () => void;
  /** Open Backups Settings…: Settings at oxyos-settings://backups. */
  openSettings: () => void;
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
  /** The networks as the session tells them (docs/adr/0038-networks.mdx): the Control Center's
      Wi-Fi view. Absent or null, the Wi-Fi module only turns Wi-Fi on and off. */
  networks?: NetworkState | null;
  /** The Wi-Fi view asks the session to scan: as it opens, and every ten seconds while open. */
  scanNetworks?: () => void;
  /** A network chosen: the session joins it, asking its password in a system sheet if it needs
      one, and says what failed in another. */
  joinNetwork?: (ssid: string) => void;
  /** The Wi-Fi view's "Ajustes de Wi-Fi…": Settings, at its Wi-Fi page where it can be. */
  openWifiSettings?: () => void;
  /** The sound's outputs and inputs as the session tells them (docs/adr/0056-sound-devices.mdx):
      the Control Center's Sound view. Absent or null, the Sound card only sets the volume. */
  sound?: SoundState | null;
  /** An output chosen: the session makes it the one played to. */
  selectOutput?: (id: string) => void;
  /** The Sound view's "Ajustes de sonido…": Settings, at its Sound page where it can be. */
  openSoundSettings?: () => void;
  /** Bluetooth as the session tells it (docs/adr/0055-bluetooth.mdx): the Bluetooth module's
      switch and the Control Center's Bluetooth view. Absent or null, the module only says what
      `bluetooth` says. */
  bluetoothDevices?: BluetoothState | null;
  /** Bluetooth's switch: the session turns the adapter on or off. */
  setBluetooth?: (on: boolean) => void;
  /** A paired device pressed in the Bluetooth view: the session connects it (asking what a
      pairing needs, and saying what failed, in its system sheets) or disconnects it. */
  connectBluetooth?: (id: string) => void;
  disconnectBluetooth?: (id: string) => void;
  /** The Bluetooth view's "Bluetooth Settings…": Settings, at its Bluetooth page. */
  openBluetoothSettings?: () => void;
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

/**
 * AutoFill as the platform has it (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6):
 * the list the session tells the shell to draw at a field's caret, in the same window as an input
 * method's candidates, and the press that picks an item. The application never sees the list.
 */
export type ShellAutofill = {
  /** The list to show; null while there is none. */
  list: AutofillList | null;
  /** An item pressed (its index in the list): the session fills it. */
  choose: (index: number) => void;
};

/**
 * Input methods as the platform has them (docs/adr/0041-input-methods.mdx): the page of candidates
 * an input method hands the system, which the shell draws at the caret, and the person's input
 * sources, which the top bar shows when there is more than one.
 */
export type ShellInputMethod = {
  /** The page to show; null while there is none (the window is hidden). */
  candidates: CandidateList | null;
  /** A candidate pressed (its index on the page): the input method chooses it. */
  choose: (index: number) => void;
  /** A chevron pressed: the page after (`forward`), or before. */
  turnPage: (forward: boolean) => void;
  /** The input sources as the session tells them; null until it has. */
  sources: InputSourcesState | null;
  /** A source chosen in the top bar's menu. */
  selectSource: (id: string) => void;
  /** "Show Emoji & Symbols": the session's emoji picker, in the field being typed in. */
  showEmoji: () => void;
  /** "Open Keyboard Settings…": Settings, at its Keyboard page where it can be. */
  openKeyboardSettings: () => void;
};
