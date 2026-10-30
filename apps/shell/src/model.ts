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
};
/** Requests go to the platform owner. Rendering a shell never launches a fake app or manages native windows itself. */
export type ShellActions = {
  activateApp: (id: string) => void;
  focusWindow: (id: string) => void;
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
    | "switcher-cancel";
};

/** Whether `window` is the application `appId`'s: its id, given by the session's launch record
    or (when the platform says nothing of launch records) its own. A window the session did not
    start is no registry application's, whatever it claims. */
export function belongsTo(window: ShellWindow, appId: string): boolean {
  return window.launched !== false && window.appId === appId;
}
