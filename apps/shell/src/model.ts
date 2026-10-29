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
  /** Pixel dimensions of the supplied thumbnail; preserves the surface aspect ratio. */
  thumbnailSize?: { width: number; height: number };
  minimized?: boolean;
};
export type ShellMenu = {
  label: string;
  items: readonly { id: string; label: string; enabled?: boolean }[];
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
};
/**
 * The session's lock (docs/adr/0010-lock-screen.mdx), owned by the platform: whether it is
 * locked, whom it is locked for, and how the lock screen asks. Only the session unlocks.
 */
export type ShellLock = {
  locked: boolean;
  user: { name: string; fullName?: string };
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
export type ShellCommand = {
  id: number;
  action: "dismiss" | "launcher" | "overview";
};
