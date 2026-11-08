import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AccessibilityInfo, StyleSheet, useColorScheme } from "react-native";
import { BloomScope } from "@oxy.so/bloom/appearance";
import { Box } from "./primitives";
import type {
  ShellActions,
  ShellCommand,
  ShellControls,
  ShellInputMethod,
  ShellBatteryMenu,
  ShellAutofill,
  ShellLock,
  ShellMenu,
  ShellNotification,
  ShellNotifications,
  ShellScreenshots,
  ShellSnapshot,
  ShellWallpaper,
} from "./model";
import { Wallpaper } from "./components/Wallpaper";
import { Desktop, type DesktopCommand, type DesktopState } from "./components/Desktop";
import { TopBar } from "./components/TopBar";
import { DOCK_HEIGHT, Dock } from "./components/Dock";
import { Overview } from "./components/Overview";
import { AppExpose, AppExposeBackdrop } from "./components/AppExpose";
import { exposedWindows } from "./expose";
import { dockPress } from "./dock";
import { Launchpad, LaunchpadBackdrop } from "./components/Launchpad";
import { Spotlight } from "./components/Spotlight";
import { LockScreen } from "./components/LockScreen";
import { AppSwitcher } from "./components/AppSwitcher";
import { Bezel } from "./components/Bezel";
import { ControlCenter, type ControlCenterPage } from "./components/ControlCenter";
import { BANNER_WIDTH, NotificationCard } from "./components/NotificationBanner";
import { NotificationCenter } from "./components/NotificationCenter";
import { SystemSheet } from "./components/SystemSheet";
import { Screenshots } from "./components/Screenshots";
import { CandidateWindow } from "./components/CandidateWindow";
import { AutofillWindow } from "./components/AutofillWindow";
import { sourceName, switchedSource, type InputSourcesState } from "./input-sources";
import type { SheetAction, SheetText } from "./sheet";
import { useShellMessages } from "./useShellMessages";
import type { ShellMessages } from "./messages";
import { hasWifiView } from "./network";
import { firstStatusItem } from "./components/TopBar";
import { applicationOf, BANNER_MS, nextBanner, spoken } from "./notifications";
import {
  chosen,
  followSwitcher,
  moveSwitcher,
  openSwitcher,
  type Switcher,
} from "./switcher";
import { inPlace, type ShellPlatform } from "./platform";
export type { ShellCommand } from "./model";
export type {
  MinimizeTargetProps,
  ScrollTargetProps,
  ShellPlatform,
  ShellSurfaceProps,
  WindowViewProps,
} from "./platform";

/** The top bar's height (TopBar's `h-7`): what banners and the Notification Center begin below. */
const TOP_BAR_HEIGHT = 28;

/** Where a region over the whole output is (the switcher, the overlays). */
const WHOLE = StyleSheet.absoluteFill;
const SWITCHER_AREA = {
  position: "absolute",
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  alignItems: "center",
  justifyContent: "center",
} as const;
const BANNER_AREA = { position: "absolute", top: TOP_BAR_HEIGHT + 8, right: 12, width: BANNER_WIDTH } as const;
const NO_LEVELS: ShellControls = { levels: {} };
/** The candidate window's region: the compositor places its surface at the caret, never here; at
    whole pixels, so it is drawn on the pixel grid. With nothing to show it holds a point, so its
    surface (and what the screen reader holds of it) stays from one list to the next. */
const CANDIDATES_AREA = { position: "absolute", top: 0, left: 0 } as const;
const NO_CANDIDATES = { width: 1, height: 1 } as const;
const NO_DESKTOP: DesktopState = { keyboard: false, selection: false, items: false };

/**
 * The bar's menus while the desktop has the keyboard (docs/adr/0047-finder-drags-and-the-desktop.mdx):
 * the shell's, with File (Open, Move to Trash) and Edit (Select All) after its first, as Finder's
 * are while its desktop is active. Their chords are the desktop's own keys (Ctrl, the applications'
 * ⌘ on OxyOS), shown as the bar shows an application's.
 */
function withDesktopMenus(menus: readonly ShellMenu[], state: DesktopState, messages: ShellMessages): ShellMenu[] {
  const words = messages.desktop;
  const desktop: ShellMenu[] = [
    {
      label: words.file,
      items: [
        { id: "desktop.open", label: words.open, enabled: state.selection, shortcut: "Ctrl+O" },
        { kind: "separator", id: "desktop.file-separator" },
        { id: "desktop.trash", label: words.moveToTrash, enabled: state.selection, shortcut: "Ctrl+Backspace" },
      ],
    },
    { label: words.edit, items: [{ id: "desktop.select-all", label: words.selectAll, enabled: state.items, shortcut: "Ctrl+A" }] },
  ];
  return [...menus.slice(0, 1), ...desktop, ...menus.slice(1)];
}

/**
 * OxyOS shell presentation. Native application surfaces belong to the compositor beneath it.
 *
 * Each region renders again only when what it shows changes: the regions are memoized, and what
 * the shell gives them (its handlers, the parts of the snapshot each reads) stays the same while it
 * does. A platform that keeps its snapshot's unchanged parts as they were (a window whose size alone
 * changed leaves the applications and the menus as they were) has the shell render nothing for
 * what shows nothing, and of the rest only the regions that show it.
 */
export const Shell = memo(function Shell({
  snapshot,
  actions,
  wallpaper,
  now,
  command,
  commands,
  lock,
  platform = inPlace,
  dock = true,
  controls,
  notifications,
  banners = true,
  sheet,
  screenshots,
  inputMethod,
  battery,
  autofill,
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  /** What the wallpaper shows (docs/adr/0048-wallpapers.mdx). */
  wallpaper: ShellWallpaper;
  now: Date;
  command?: ShellCommand;
  /** The latest commands, each with a rising `id`: every one not handled yet is, in order (the
      system shortcuts, which may come several at once, docs/adr/0024-system-shortcuts.mdx). */
  commands?: readonly ShellCommand[];
  /** The session's lock, where the platform has one: the lock screen, and the menu's item. */
  lock?: ShellLock;
  /** Where the regions are drawn: separate surfaces on OxyOS, in place elsewhere. */
  platform?: ShellPlatform;
  /**
   * Whether this display has the dock: the primary has it, as a Mac's main display has the Dock;
   * every display has the top bar (docs/adr/0018-several-displays.mdx).
   */
  dock?: boolean;
  /**
   * The machine as the session says it is, and the Control Center's intents
   * (docs/adr/0025-hardware-keys-and-control-center.mdx): the top bar's battery and Wi-Fi, the
   * Control Center, and the bezel of a hardware key. Without it, none is shown but the
   * Control Center's appearance.
   */
  controls?: ShellControls;
  /** The session's notifications, where the platform has them (docs/adr/0026-notifications.mdx):
      banners, the Notification Center from the clock, and the lock screen's summary. */
  notifications?: ShellNotifications;
  /** Whether this display shows banners: the primary does, as a Mac shows them on its main one. */
  banners?: boolean;
  /** The session's system sheet to show now (ADR 0030): the first waiting, and its answer. */
  sheet?: {
    /** The session's number for it: a sheet asked again (a Wi-Fi password that was wrong) is a
        new one, its field empty. */
    id?: number;
    text: SheetText;
    names?: Record<string, string>;
    answer: (accepted: boolean) => void;
    /** A Wi-Fi password sheet's answer (docs/adr/0038-networks.mdx): the password, to the
        session alone. Without it such a sheet cannot be answered but by Cancel. */
    answerPassword?: (password: string) => void;
    /** A Bluetooth pairing code's answer (docs/adr/0055-bluetooth.mdx): the passkey or PIN typed,
        to the session alone. Without it such a sheet cannot be answered but by Cancel. */
    answerCode?: (code: string) => void;
    /** A passkey sheet's answer (docs/adr/0050-keychain-autofill-and-passkeys.mdx): the passkey
        chosen and the Oxy account's password, to the session alone. Without it such a sheet
        cannot be answered but by Cancel. */
    answerPasskey?: (choice: number | undefined, password: string) => void;
    /** What else a sheet offers, pressed (an application that quit: Reset…, which asks the session
        to reset it, docs/adr/0061-crashes-and-resetting-an-application.mdx). */
    act?: (action: SheetAction) => void;
    /** Puts a report's text on the clipboard, at the person's press. */
    copy?: (text: string) => Promise<boolean> | boolean | void;
  };
  /** Screenshots, where the platform takes them (docs/adr/0024-system-shortcuts.mdx, decision 10):
      the interface the compositor opened from the keyboard, and the thumbnail of one saved. */
  screenshots?: ShellScreenshots;
  /** Input methods, where the platform has them (docs/adr/0041-input-methods.mdx): the candidate
      window at the caret, and the top bar's input source item. */
  inputMethod?: ShellInputMethod;
  /** The battery's menu, where the platform has the session's battery and energy
      (docs/adr/0057-battery-and-energy.mdx). */
  battery?: ShellBatteryMenu;
  /** AutoFill, where the platform has it (docs/adr/0050-keychain-autofill-and-passkeys.mdx): its
      list at a field's caret, drawn in the candidate window's surface. */
  autofill?: ShellAutofill;
}) {
  const { Surface } = platform;
  const messages = useShellMessages();
  const dark = useColorScheme() === "dark";
  // The Control Center, open at its modules or at Wi-Fi's view (docs/adr/0038-networks.mdx).
  const [controlCenter, setControlCenter] = useState<ControlCenterPage | null>(null);
  const shownControls: ShellControls = controls ?? NO_LEVELS;
  // The lock closes it, as it closes every other panel.
  useEffect(() => {
    if (lock?.locked) setControlCenter(null);
  }, [lock?.locked]);
  // The Notification Center, from the clock (docs/adr/0026-notifications.mdx).
  const [center, setCenter] = useState(false);
  // The banner shown, a notification that arrived to be one; each is shown once.
  const [banner, setBanner] = useState<ShellNotification | null>(null);
  // Whether the person has the banner (the pointer over it, a reply being typed in it).
  const [bannerHeld, setBannerHeld] = useState(false);
  const seen = useRef<Set<string> | null>(null);
  const list = notifications?.list;
  useEffect(() => {
    if (!list) return;
    // What the shell knew when it started is no banner.
    const first = seen.current === null;
    const known = seen.current ?? new Set<string>();
    const next = first ? undefined : nextBanner(list, known);
    for (const notification of list) known.add(notification.id);
    seen.current = known;
    if (next && banners && !lock?.locked && !notifications?.doNotDisturb) {
      setBanner(next);
      setBannerHeld(false);
      // Said politely, after what the screen reader is saying, as VoiceOver says a banner.
      const said = spoken(next, applicationOf(next, snapshot.applications).name);
      if (platform.announce) platform.announce(said, true);
      else AccessibilityInfo.announceForAccessibility(said);
    } else {
      // One no longer kept (opened, removed) goes with its banner.
      setBanner((shown) => (shown && list.some((notification) => notification.id === shown.id) ? shown : null));
    }
  }, [list]);
  // The Notification Center opened takes the banner shown into it, as a Mac's does.
  useEffect(() => {
    if (center) setBanner(null);
  }, [center]);
  // A banner stays five seconds, and while the person has it, as a Mac's stays while the pointer
  // is on it; let go, it stays five seconds more.
  useEffect(() => {
    if (!banner || bannerHeld) return undefined;
    const timer = setTimeout(() => setBanner(null), BANNER_MS);
    return () => clearTimeout(timer);
  }, [banner, bannerHeld]);
  // What covers the desktop: Launchpad, the overview (Mission Control) or App Exposé
  // (docs/adr/0045-app-expose.mdx), whose application is `exposeApp`.
  const [view, setView] = useState<"desktop" | "launcher" | "overview" | "app-expose">(
    "desktop",
  );
  const [escapeRequest, setEscapeRequest] = useState(0);
  const statusKeyboardLostRef = useRef(() => {});
  const [searchRequest, setSearchRequest] = useState(0);
  // The top bar from the keyboard (Super+F2, docs/adr/0024-system-shortcuts.mdx): each request,
  // whether its surface wants the keyboard, and each time the keyboard left it.
  const [barRequest, setBarRequest] = useState(0);
  const [barKeyboard, setBarKeyboard] = useState(false);
  const [barLost, setBarLost] = useState(0);
  // Full Keyboard Access (docs/adr/0039-keyboard-access.mdx): the dock (Super+F3, a Mac's
  // Control-F3) and the top bar's status items (Super+F8, its Control-F8) with the keyboard, each
  // the control its focus goes to and a request that rises each time it is asked again; the
  // Control Center's, its first control once it opened from the keyboard.
  const [dockKeys, setDockKeys] = useState<{ target: string; request: number } | null>(null);
  const [statusKeys, setStatusKeys] = useState<{ target: string; request: number } | null>(null);
  const [controlKeys, setControlKeys] = useState(0);
  // The Notification Center's, its first control once it opened from the keyboard (the clock, or
  // Super+N).
  const [centerKeys, setCenterKeys] = useState(0);
  // The overview's, its first control once it opened from the keyboard (the dock's button).
  const [overviewKeys, setOverviewKeys] = useState(0);
  // App Exposé's: the window the keyboard's focus goes to, asked again with each request.
  const [exposeKeys, setExposeKeys] = useState<{ target: string; request: number } | null>(null);
  // Spotlight (docs/adr/0051-search.mdx): whether it is open, each request for its field to take
  // the keyboard, and each Super+Return that reveals its selected result.
  const [spotlight, setSpotlight] = useState(false);
  const [spotlightKeys, setSpotlightKeys] = useState(0);
  const [revealRequest, setRevealRequest] = useState(0);
  // The application switcher (docs/adr/0024-system-shortcuts.mdx), open while Super is held.
  const [switcher, setSwitcher] = useState<Switcher | null>(null);
  // Commands act on what is shown now, whichever render they arrive in.
  const inputSources = inputMethod?.sources ?? null;
  const latest = useRef({ snapshot, actions, switcher, view, controls, statusKeys, dockKeys, center, notifications, inputSources, battery, spotlight });
  latest.current = { snapshot, actions, switcher, view, controls, statusKeys, dockKeys, center, notifications, inputSources, battery, spotlight };
  // The application App Exposé shows (docs/adr/0045-app-expose.mdx): the dock menu's Show All
  // Windows, a scroll up on its dock icon, Super+↓ and the touchpad's swipe down.
  const [exposeApp, setExposeApp] = useState<string | null>(null);
  const exposing = useRef<string | null>(null);
  exposing.current = view === "app-expose" ? exposeApp : null;
  const dismiss = useCallback(() => {
    setView("desktop");
    setExposeApp(null);
  }, []);
  const toggleOverview = useCallback(() => {
    // Opened from the dock with the keyboard, it takes the keyboard on its first control, as
    // Mission Control opened from the keyboard does.
    if (latest.current.dockKeys) setOverviewKeys((value) => value + 1);
    setExposeApp(null);
    setView((current) => (current === "overview" ? "desktop" : "overview"));
  }, []);
  const toggleLauncher = useCallback(() => {
    setExposeApp(null);
    setView((current) => (current === "launcher" ? "desktop" : "launcher"));
  }, []);
  // App Exposé for an application with windows, open or minimized (one with none has nothing to
  // show, as a Mac's Dock does nothing then); the keyboard on its frontmost window.
  const exposes = (id: string) => {
    const { open, minimized } = exposedWindows(latest.current.snapshot, id);
    return open.length + minimized.length > 0;
  };
  const firstExposed = (id: string) => {
    const { open, minimized } = exposedWindows(latest.current.snapshot, id);
    const first = open[0] ?? minimized[0];
    return first ? `expose-window-${first.id}` : "first";
  };
  const showAllWindows = useCallback((id: string) => {
    if (!exposes(id)) return;
    setExposeApp(id);
    setView("app-expose");
    setExposeKeys((keys) => ({ target: firstExposed(id), request: (keys?.request ?? 0) + 1 }));
  }, []);
  const moveExposeFocus = useCallback(
    (target: string) => setExposeKeys((keys) => ({ target, request: (keys?.request ?? 0) + 1 })),
    [],
  );
  // A scroll on a dock icon (docs/adr/0045-app-expose.mdx): up opens the application's App
  // Exposé, as a Mac's Dock can; down closes it.
  const scrollIcon = useCallback(
    (id: string, up: boolean) => {
      if (up) {
        if (exposing.current !== id) showAllWindows(id);
      } else if (exposing.current === id) {
        dismiss();
      }
    },
    [showAllWindows, dismiss],
  );
  const activateApp = useCallback(
    (id: string) => {
      // In App Exposé another application's icon shows its windows instead, as a Mac's does.
      if (exposing.current && exposing.current !== id && exposes(id)) {
        showAllWindows(id);
        return;
      }
      latest.current.actions.activateApp(id);
      setView("desktop");
    },
    [showAllWindows],
  );
  // The windows each application's icon pressed last minimized (docs/adr/0034-the-dock.mdx,
  // decision 9), the one that was frontmost last: pressed again, they come back together.
  const minimizedByDock = useRef(new Map<string, string[]>());
  // A press on an application's icon in the dock (docs/adr/0034-the-dock.mdx, decision 9): what a
  // Mac's Dock does, and the frontmost application's windows minimized into its icon and restored
  // out of it again (the owner's decision); Option hides the others.
  const pressDockApp = useCallback(
    (id: string, option: boolean) => {
      const now = latest.current;
      // In App Exposé another application's icon shows its windows instead, as a Mac's does.
      if (exposing.current && exposing.current !== id && exposes(id)) {
        showAllWindows(id);
        return;
      }
      const press = dockPress(now.snapshot, id, {
        option,
        desktop: now.view === "desktop",
        minimizedByDock: minimizedByDock.current.get(id),
      });
      switch (press.kind) {
        case "open":
          now.actions.activateApp(id);
          break;
        case "minimize":
          if (!now.actions.minimizeWindow) {
            now.actions.activateApp(id);
            break;
          }
          minimizedByDock.current.set(id, press.windows);
          for (const window of press.windows) now.actions.minimizeWindow(window);
          break;
        case "hide":
          if (now.actions.hideApp) now.actions.hideApp(id);
          else now.actions.activateApp(id);
          break;
        case "restore":
          minimizedByDock.current.delete(id);
          for (const app of press.hide) now.actions.hideApp?.(app);
          // Each restored in turn, the one used last last: it is frontmost, with the keyboard.
          for (const window of press.windows) now.actions.focusWindow(window);
          break;
        case "activate":
          for (const app of press.hide) now.actions.hideApp?.(app);
          now.actions.activateApp(id);
          break;
      }
      setView("desktop");
      setExposeApp(null);
    },
    [showAllWindows],
  );
  // Each command once, in order: several may come in one render (Super+Tab released at once
  // opens the switcher and ends it with its choice, as a Mac's quick ⌘Tab switches back).
  const done = useRef(0);
  useEffect(() => {
    const pending = [...(commands ?? []), ...(command ? [command] : [])]
      .filter((next) => next.id > done.current)
      .sort((a, b) => a.id - b.id);
    if (pending.length === 0) return;
    done.current = pending[pending.length - 1].id;
    const now = latest.current;
    let current = now.switcher;
    let view = now.view;
    let expose: string | null = null;
    let spotted = now.spotlight;
    for (const next of pending) {
      switch (next.action) {
        case "dismiss":
          setEscapeRequest((value) => value + 1);
          break;
        case "launcher":
        case "overview":
          view = next.action;
          break;
        // Super+↑: Mission Control; again, or from App Exposé, it closes
        // (docs/adr/0045-app-expose.mdx). Three or four fingers swiped up open it as they begin to
        // drive it, and close it if they let it go back; the swipe down closes it
        // (docs/adr/0046-interactive-gestures.mdx).
        case "mission-control":
          if (next.intent === "open") view = "overview";
          else if (next.intent === "close") view = view === "overview" ? "desktop" : view;
          else view = view === "desktop" || view === "launcher" ? "overview" : "desktop";
          break;
        // Super+↓: App Exposé of the frontmost application; again, or from Mission Control, it
        // closes. The swipe down opens it, the swipe up closes it.
        case "app-expose": {
          const front = now.snapshot.activeAppId;
          const opens = () => {
            if (front && exposes(front)) {
              view = "app-expose";
              expose = front;
            }
          };
          if (next.intent === "open") {
            if (view !== "app-expose") opens();
          } else if (next.intent === "close") {
            if (view === "app-expose") view = "desktop";
          } else if (view === "overview" || view === "app-expose") {
            view = "desktop";
          } else {
            opens();
          }
          break;
        }
        // Super pressed alone (ADR 0041; Super+Space before it): the launcher, its search taking what is typed; again, it closes.
        // The thumb and three fingers pinched open it as they begin to drive it, and close it if
        // they let it go back; spread over it, they close it (docs/adr/0046-interactive-gestures.mdx,
        // amended 8 November 2026).
        case "search":
          if (next.intent === "close") {
            if (view === "launcher") view = "desktop";
          } else if (view === "launcher" && next.intent !== "open") {
            view = "desktop";
          } else if (view !== "launcher") {
            view = "launcher";
            setSearchRequest((value) => value + 1);
          }
          break;
        // Super+Space, as a Mac's ⌘Space: Spotlight, its field taking what is typed; again, it
        // closes, the keyboard going back to the window. Opening it closes what covers the desktop
        // and the status panels, as a Mac's Spotlight closes Launchpad and Mission Control.
        case "spotlight":
          if (spotted) {
            spotted = false;
            now.actions.returnKeyboard?.();
          } else {
            spotted = true;
            view = "desktop";
            setControlCenter(null);
            setCenter(false);
            setDockKeys(null);
            setStatusKeys(null);
            setSpotlightKeys((value) => value + 1);
          }
          break;
        // Super+Return while Spotlight is open, as a Mac's ⌘Return: its selected result in Dropsea.
        case "reveal":
          if (spotted) setRevealRequest((value) => value + 1);
          break;
        // Super+F2: the keyboard to the top bar's first menu; an overlay showing closes, as a
        // press on the bar closes it.
        case "menu-bar":
          view = "desktop";
          setDockKeys(null);
          setStatusKeys(null);
          setBarRequest((value) => value + 1);
          break;
        // Super+F3: the keyboard to the dock's first icon; ← → go along it, ↑ opens an icon's
        // menu, Space and Return open it, Escape gives the window the keyboard back.
        case "dock":
          view = "desktop";
          setStatusKeys(null);
          setDockKeys((keys) => ({ target: "first", request: (keys?.request ?? 0) + 1 }));
          break;
        // Super+F8: the keyboard to the top bar's first status item; Space opens it.
        case "status-menus":
          view = "desktop";
          setDockKeys(null);
          setControlCenter(null);
          setCenter(false);
          setStatusKeys((keys) => ({
            target: firstStatusItem(
              now.controls?.network,
              now.inputSources,
              !!(now.battery && now.controls?.power && now.controls.power.battery !== "absent"),
            ),
            request: (keys?.request ?? 0) + 1,
          }));
          break;
        // Super+N, as a Mac's Fn-N: the Notification Center, with the keyboard on its first
        // control; again, it closes.
        case "notifications":
          if (!now.notifications) break;
          view = "desktop";
          setDockKeys(null);
          setStatusKeys(null);
          setControlCenter(null);
          if (!now.center) setCenterKeys((value) => value + 1);
          setCenter(!now.center);
          break;
        case "switcher-next":
        case "switcher-previous": {
          const backwards = next.action === "switcher-previous";
          current = current
            ? moveSwitcher(current, backwards ? -1 : 1)
            : openSwitcher(now.snapshot, backwards);
          break;
        }
        // Its choice is quit or hidden; it stays, and follows what still runs.
        case "switcher-quit":
          if (current) now.actions.quitApp?.(chosen(current));
          break;
        case "switcher-hide":
          if (current) now.actions.hideApp?.(chosen(current));
          break;
        case "switcher-done":
          if (current) {
            now.actions.activateApp(chosen(current));
            view = "desktop";
          }
          current = null;
          break;
        case "switcher-cancel":
          current = null;
          break;
      }
    }
    setSwitcher(current);
    setSpotlight(spotted);
    if (view === "app-expose" && expose) {
      const shown = expose;
      setExposeApp(shown);
      setExposeKeys((keys) => ({ target: firstExposed(shown), request: (keys?.request ?? 0) + 1 }));
    } else if (view !== "app-expose") {
      setExposeApp(null);
    }
    setView(view);
  }, [command, commands]);
  // What covers the desktop, a status panel or the lock closes Spotlight, as each takes its place.
  useEffect(() => {
    if (view !== "desktop" || controlCenter || center || lock?.locked) setSpotlight(false);
  }, [view, controlCenter, center, lock?.locked]);
  // Closed by Escape or by Super+Space, the keyboard goes back to the window; by what it opened, or
  // by a press elsewhere, it is already where it goes.
  const closeSpotlight = useCallback((dismissed: boolean) => {
    setSpotlight(false);
    if (dismissed) latest.current.actions.returnKeyboard?.();
  }, []);
  // An application quit while the switcher is open leaves it.
  useEffect(() => {
    setSwitcher((current) => current && followSwitcher(current, snapshot));
  }, [snapshot]);
  // A window or a desktop chosen in the overview is where the keyboard goes, as in Mission
  // Control: the dock it was opened from with the keyboard lets it go. Closed without a choice
  // (Escape), the keyboard goes back to the dock.
  // A window chosen has the keyboard: the overlay closing does not give it back to the window
  // that had it before (`windowChosen`).
  const windowChosen = useRef(false);
  const overviewActions = useMemo(
    () => ({
      ...actions,
      focusWindow: (id: string) => {
        setDockKeys(null);
        windowChosen.current = true;
        actions.focusWindow(id);
      },
      activateWorkspace: (id: string) => {
        setDockKeys(null);
        actions.activateWorkspace(id);
      },
    }),
    [actions],
  );
  const launcherOpen = view === "launcher";
  const desktopArea = useMemo(
    () => ({ position: "absolute", top: TOP_BAR_HEIGHT, left: 0, right: 0, bottom: dock ? DOCK_HEIGHT : 0 }) as const,
    [dock],
  );
  const exposeArea = useMemo(
    () => ({ position: "absolute", top: 0, left: 0, right: 0, bottom: dock ? DOCK_HEIGHT : 0 }) as const,
    [dock],
  );
  const launcherArea = useMemo(
    () =>
      ({
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        bottom: dock ? DOCK_HEIGHT : 0,
        display: launcherOpen ? "flex" : "none",
      }) as const,
    [dock, launcherOpen],
  );
  const lockSession = lock?.lockSession;
  // The desktop (docs/adr/0047-finder-drags-and-the-desktop.mdx): what the bar's menus show of it,
  // and what they ask of it.
  const [desktop, setDesktop] = useState<DesktopState>(NO_DESKTOP);
  const [desktopCommand, setDesktopCommand] = useState<DesktopCommand | undefined>(undefined);
  const menuCommand = useCallback(
    (id: string) => {
      if (id === "desktop.open" || id === "desktop.trash" || id === "desktop.select-all") {
        const action = id === "desktop.open" ? "open" : id === "desktop.trash" ? "trash" : "select-all";
        setDesktopCommand((last) => ({ id: (last?.id ?? 0) + 1, action }));
        return;
      }
      if (id === "view.launcher" || id === "view.overview") setExposeApp(null);
      if (id === "view.launcher") setView("launcher");
      else if (id === "view.overview") setView("overview");
      else if (id === "session.lock") lockSession?.();
      else latest.current.actions.menuCommand(id);
    },
    [lockSession],
  );
  const keyboardLost = useCallback(() => {
    setBarLost((value) => value + 1);
    statusKeyboardLostRef.current();
  }, []);
  // One of the two at a time, as a Mac's Control Center and Notification Center.
  // Opened while the keyboard is on the status items, the Control Center takes it on its first
  // control, as a Mac's status menu opened from Control-F8 does.
  const fromKeyboard = () => {
    if (latest.current.statusKeys) setControlKeys((value) => value + 1);
  };
  const toggleControlCenter = useCallback(() => {
    setCenter(false);
    fromKeyboard();
    setControlCenter((open) => (open ? null : "modules"));
  }, []);
  // The top bar's network item opens it at Wi-Fi's view, or closes it, as a Mac's Wi-Fi menu;
  // where there is no Wi-Fi view (no networks told, no Wi-Fi device), at its modules.
  const wifiView = hasWifiView(controls?.networks);
  const openNetworkStatus = useCallback(() => {
    setCenter(false);
    fromKeyboard();
    setControlCenter((open) => (open ? null : wifiView ? "wifi" : "modules"));
  }, [wifiView]);
  const hasNotifications = !!notifications;
  const toggleCenter = useMemo(
    () =>
      hasNotifications
        ? () => {
            setControlCenter(null);
            if (latest.current.statusKeys) setCenterKeys((value) => value + 1);
            setCenter((open) => !open);
          }
        : undefined,
    [hasNotifications],
  );
  const closeControlCenter = useCallback(() => setControlCenter(null), []);
  // What each status item opens, shown or not: the network's item opens Control Center at its
  // Wi-Fi view (at its modules where there is none), Control Center's item it at any view.
  const controlCenterOpen = controlCenter !== null;
  const networkOpen = controlCenterOpen && controlCenter === (wifiView ? "wifi" : "modules");
  const notificationsOpen = center && !!notifications;
  const statusPanels = useMemo(
    () => ({ network: networkOpen, controlCenter: controlCenterOpen, notifications: notificationsOpen }),
    [networkOpen, controlCenterOpen, notificationsOpen],
  );
  // Moving between its modules and Wi-Fi's view from the keyboard keeps the keyboard in it, on
  // the new page's first control (the control it was on is gone).
  const showControlPage = useCallback((page: ControlCenterPage | null) => {
    setControlKeys((value) => (value > 0 ? value + 1 : value));
    setControlCenter(page);
  }, []);
  useEffect(() => {
    if (!controlCenter) setControlKeys(0);
  }, [controlCenter]);
  useEffect(() => {
    if (!center) setCenterKeys(0);
  }, [center]);
  useEffect(() => {
    if (view !== "overview") setOverviewKeys(0);
    if (view !== "app-expose") setExposeKeys(null);
  }, [view]);
  // The top bar, the dock and the overlays with the keyboard hear Escape and their own keys
  // (docs/adr/0039-keyboard-access.mdx): an overlay closes, the dock and the status items give
  // the window back the keyboard. A sheet over them takes its own keys.
  const keyboard = platform.keyboard;
  // The top bar's input source menu open: Escape is its own, closing it.
  const inputMenu = useRef(false);
  const inputMenuChanged = useCallback((open: boolean) => {
    inputMenu.current = open;
  }, []);
  const keys = useRef({ controlCenter, center, view, statusKeys, dockKeys, sheet: !!sheet });
  keys.current = { controlCenter, center, view, statusKeys, dockKeys, sheet: !!sheet };
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      const now = keys.current;
      if (now.sheet || inputMenu.current || event.key !== "Escape") return false;
      if (now.controlCenter) {
        setControlCenter(null);
        return true;
      }
      if (now.center) {
        setCenter(false);
        return true;
      }
      if (now.view === "overview" || now.view === "app-expose") {
        dismiss();
        return true;
      }
      if (now.statusKeys) {
        setStatusKeys(null);
        latest.current.actions.returnKeyboard?.();
        return true;
      }
      return false;
    });
  }, [keyboard, dismiss]);
  // An overlay that had the keyboard closing, however it closes (Escape, a press outside it, a
  // choice in it), gives it back to the window, or to the status items it opened from.
  const overlayOpen = !!controlCenter || center || view === "overview" || view === "app-expose";
  const wasOpen = useRef(overlayOpen);
  useEffect(() => {
    const closed = wasOpen.current && !overlayOpen;
    wasOpen.current = overlayOpen;
    const choice = windowChosen.current;
    if (closed) windowChosen.current = false;
    if (closed && !choice && !keys.current.statusKeys && !keys.current.sheet && keys.current.view !== "launcher") {
      latest.current.actions.returnKeyboard?.();
    }
  }, [overlayOpen]);
  const leaveDock = useCallback(() => {
    setDockKeys(null);
    latest.current.actions.returnKeyboard?.();
  }, []);
  const dockKeyboardLost = useCallback(() => setDockKeys(null), []);
  const moveDockFocus = useCallback(
    (target: string) => setDockKeys((keys) => (keys ? { target, request: keys.request + 1 } : keys)),
    [],
  );
  const moveStatusFocus = useCallback(
    (target: string) => setStatusKeys((keys) => (keys ? { target, request: keys.request + 1 } : keys)),
    [],
  );
  const leaveStatus = useCallback(() => {
    setStatusKeys(null);
    latest.current.actions.returnKeyboard?.();
  }, []);
  statusKeyboardLostRef.current = () => {
    // The keyboard went to the Control Center the status items opened: they keep their place.
    if (!keys.current.controlCenter) setStatusKeys(null);
  };
  // Wi-Fi joined is said, as VoiceOver hears a Mac's Wi-Fi menu join: once, as it connects.
  const network = controls?.network;
  const joined = network?.wifi === "connected" ? network.ssid : "";
  const saidJoined = useRef(joined);
  useEffect(() => {
    if (joined === saidJoined.current) return;
    saidJoined.current = joined;
    if (!joined) return;
    const said = messages.wifi.joined(joined);
    if (platform.announce) platform.announce(said, true);
    else AccessibilityInfo.announceForAccessibility(said);
  }, [joined]);
  // Each switch of input source is said, politely, by the new source's name, as VoiceOver says a
  // Mac's (docs/adr/0041-input-methods.mdx, decision 6): not the first state told, nor a change of
  // the list that moved the source in use, which Settings says itself (`switchedSource`).
  const toldSources = useRef<InputSourcesState | null>(inputSources ?? null);
  useEffect(() => {
    const before = toldSources.current;
    toldSources.current = inputSources ?? null;
    const switched = switchedSource(before, inputSources ?? null);
    if (!inputSources || switched === null) return;
    const said = sourceName(switched, inputSources, messages);
    if (platform.announce) platform.announce(said, true);
    else AccessibilityInfo.announceForAccessibility(said);
  }, [inputSources]);
  const openSettings = useCallback(() => menuCommand("system.settings"), [menuCommand]);
  const menus = useMemo(
    () => (desktop.keyboard ? withDesktopMenus(snapshot.menus, desktop, messages) : snapshot.menus),
    [desktop, snapshot.menus, messages],
  );
  const setDoNotDisturb = notifications?.setDoNotDisturb;
  const doNotDisturbOn = !!notifications?.doNotDisturb;
  const doNotDisturb = useMemo(
    () => (setDoNotDisturb ? { on: doNotDisturbOn, set: setDoNotDisturb } : undefined),
    [doNotDisturbOn, setDoNotDisturb],
  );
  const closeCenter = useCallback(() => setCenter(false), []);
  const closeBanner = useCallback(() => setBanner(null), []);
  const activateNotification = notifications?.activate;
  const bannerId = banner?.id;
  const activateBanner = useCallback(
    (action: number, reply?: string) => {
      if (bannerId === undefined) return;
      activateNotification?.(bannerId, action, reply);
      setBanner(null);
    },
    [activateNotification, bannerId],
  );
  const activateFromSwitcher = useCallback(
    (id: string) => {
      setSwitcher(null);
      activateApp(id);
    },
    [activateApp],
  );
  return (
    <BloomScope size="md">
      <Box testID="oxy-shell" className="flex-1 overflow-hidden bg-[#294ab4]">
        <Box className="absolute inset-0">
          <Wallpaper wallpaper={wallpaper} />
        </Box>
        {/* The desktop's files (docs/adr/0047-finder-drags-and-the-desktop.mdx): beneath every
            window, between the top bar and the dock, on the display with the dock. */}
        {dock && (
          <Box pointerEvents="box-none" style={desktopArea}>
            <Desktop
              applications={snapshot.applications}
              actions={actions}
              keyboard={sheet ? undefined : keyboard}
              focusDesktop={platform.focusDesktop}
              useDesktopKeyboard={platform.useDesktopKeyboard}
              announce={platform.announce}
              command={desktopCommand}
              onState={setDesktop}
            />
          </Box>
        )}
        <Surface
          kind="panel"
          edge="top"
          exclusive
          keyboard={barKeyboard || (!!statusKeys && !controlCenter && !center && !sheet)}
          focusTarget={statusKeys?.target}
          focusRequest={statusKeys?.request}
          onKeyboardLost={keyboardLost}
        >
          <TopBar
            safeMode={snapshot.safeMode}
            keyboardRequest={barRequest}
            keyboardLost={barLost}
            onKeyboard={setBarKeyboard}
            returnKeyboard={actions.returnKeyboard}
            menus={menus}
            now={now}
            overview={toggleOverview}
            launcher={toggleLauncher}
            canLock={!!lock}
            onCommand={menuCommand}
            power={controls?.power}
            network={controls?.network}
            controlCenter={toggleControlCenter}
            networkStatus={openNetworkStatus}
            clock={toggleCenter}
            open={statusPanels}
            statusKeys={statusKeys && !controlCenter && !center && !sheet ? keyboard : undefined}
            moveStatusFocus={moveStatusFocus}
            leaveStatus={leaveStatus}
            inputSources={inputSources}
            selectInputSource={inputMethod?.selectSource}
            showEmoji={inputMethod?.showEmoji}
            openKeyboardSettings={inputMethod?.openKeyboardSettings}
            onInputMenuChange={inputMenuChanged}
            batteryMenu={battery}
          />
        </Surface>
        {(inputMethod || autofill) && (
          // What the system shows at a field's caret: AutoFill's list, or an input method's
          // candidates (docs/adr/0050-keychain-autofill-and-passkeys.mdx, ADR 0041).
          <Surface kind="candidates" style={CANDIDATES_AREA}>
            {autofill?.list ? (
              <AutofillWindow list={autofill.list} onChoose={autofill.choose} />
            ) : inputMethod?.candidates ? (
              <CandidateWindow
                list={inputMethod.candidates}
                onChoose={inputMethod.choose}
                onTurnPage={inputMethod.turnPage}
              />
            ) : (
              <Box style={NO_CANDIDATES} />
            )}
          </Surface>
        )}
        <Box
          className="min-h-0 flex-1"
          pointerEvents={view === "desktop" ? "box-none" : "auto"}
        >
          {view === "overview" && (
            // Mission Control's windows are the windows themselves, moved into its cards and
            // flying there and back as App Exposé's do, and the swipe up drives it
            // (docs/adr/0046-interactive-gestures.mdx).
            <Surface
              kind="overlay"
              style={WHOLE}
              movesWindows
              gesture="mission-control"
              keyboard={!sheet}
              focusTarget={overviewKeys > 0 ? "first" : undefined}
              focusRequest={overviewKeys}
            >
              <Overview
                snapshot={snapshot}
                actions={overviewActions}
                dismiss={dismiss}
                escapeRequest={escapeRequest}
                WindowView={platform.WindowView}
                useWindowSizes={platform.useWindowSizes}
              />
            </Surface>
          )}
        </Box>
        {/* App Exposé (docs/adr/0045-app-expose.mdx) covers the screen but the dock, as Launchpad
            does: the dimmed desktop beneath, a backdrop the compositor only fades, and over it the
            application's windows, moved into its surface (`movesWindows`), which the compositor
            flies from where they are and back. */}
        {view === "app-expose" && exposeApp && (
          <Surface kind="backdrop" style={exposeArea} gesture="app-expose">
            <AppExposeBackdrop wallpaper={wallpaper} />
          </Surface>
        )}
        {view === "app-expose" && exposeApp && (
          <Surface
            kind="overlay"
            style={exposeArea}
            movesWindows
            gesture="app-expose"
            keyboard={!sheet}
            focusTarget={exposeKeys?.target}
            focusRequest={exposeKeys?.request}
          >
            <AppExpose
              snapshot={snapshot}
              appId={exposeApp}
              actions={overviewActions}
              dismiss={dismiss}
              escapeRequest={escapeRequest}
              WindowView={platform.WindowView}
              useWindowSizes={platform.useWindowSizes}
              keyboard={sheet ? undefined : platform.keyboard}
              focus={moveExposeFocus}
              bottom={dock ? DOCK_HEIGHT : 0}
            />
          </Surface>
        )}
        {/* The launcher covers the whole screen, the top bar included, but the dock: as Launchpad
            covers the menu bar and leaves the Dock, whose button closes it again. Its blurred
            backdrop is a surface of its own beneath its icons', so the compositor fades the one
            and settles the other (docs/adr/0011-transitions.mdx). It is built once, when the
            shell starts, and kept while hidden, as macOS keeps Launchpad: opening it shows what
            is built. Hidden, it has no surfaces and takes no input. The thumb and three fingers
            pinched drive both, as far as the fingers have it (docs/adr/0046-interactive-gestures.mdx). */}
        <Surface kind="backdrop" style={launcherArea} gesture="launchpad">
          <LaunchpadBackdrop open={launcherOpen} wallpaper={wallpaper} />
        </Surface>
        <Surface kind="overlay" style={launcherArea} gesture="launchpad">
          <Launchpad
            open={launcherOpen}
            escapeRequest={escapeRequest}
            searchRequest={searchRequest}
            applications={snapshot.applications}
            folders={snapshot.folders}
            launch={activateApp}
            dismiss={dismiss}
            keyboard={platform.keyboard}
            search={platform.search}
          />
        </Surface>
        {/* Spotlight (docs/adr/0051-search.mdx): a floating panel over the windows, its own
            region of an overlay, with the keyboard while open. */}
        {spotlight && !lock?.locked && (
          <Spotlight
            platform={platform}
            applications={snapshot.applications}
            actions={actions}
            launch={activateApp}
            close={closeSpotlight}
            focusRequest={spotlightKeys}
            revealRequest={revealRequest}
            escapeRequest={escapeRequest}
            keyboard={!sheet}
          />
        )}
        {switcher && (
          <Surface kind="overlay" style={SWITCHER_AREA}>
            <AppSwitcher
              switcher={switcher}
              applications={snapshot.applications}
              activate={activateFromSwitcher}
            />
          </Surface>
        )}
        {/* The Control Center, from the top bar's button: over everything but the top bar,
            closed by a press outside it. */}
        {controlCenter && !lock?.locked && (
          // It has the keyboard while open, as a Mac's status menu does: Escape closes it, and Tab
          // goes through its controls.
          <Surface
            kind="overlay"
            style={WHOLE}
            keyboard={!sheet}
            focusTarget={controlKeys > 0 ? "first" : undefined}
            focusRequest={controlKeys}
          >
            <ControlCenter
              controls={shownControls}
              dark={dark}
              close={closeControlCenter}
              openSettings={openSettings}
              doNotDisturb={doNotDisturb}
              page={controlCenter}
              showPage={showControlPage}
            />
          </Surface>
        )}
        {/* A banner: a notification surface of its own at the top right, which the compositor slides
            in; each its own surface. Not while the Notification Center is open. */}
        {notifications && banner && !center && !lock?.locked && (
          <Surface
            key={banner.id}
            // At the right edge: a banner, which takes presses (a bezel has no edge and takes none).
            kind="notification"
            edge="right"
            style={BANNER_AREA}
          >
            <NotificationCard
              testID="notification-banner"
              notification={banner}
              applications={snapshot.applications}
              now={now}
              closeAction={messages.notifications.closeBanner}
              close={closeBanner}
              onHeld={setBannerHeld}
              activate={activateBanner}
            />
          </Surface>
        )}
        {notifications && center && !lock?.locked && (
          <Surface
            kind="overlay"
            style={WHOLE}
            keyboard={!sheet}
            focusTarget={centerKeys > 0 ? "first" : undefined}
            focusRequest={centerKeys}
          >
            <NotificationCenter
              notifications={notifications}
              applications={snapshot.applications}
              now={now}
              top={TOP_BAR_HEIGHT}
              close={closeCenter}
            />
          </Surface>
        )}
        {dock && (
          <Surface
            kind="panel"
            edge="bottom"
            exclusive
            keyboard={!!dockKeys && !sheet}
            focusTarget={dockKeys?.target}
            focusRequest={dockKeys?.request}
            onKeyboardLost={dockKeyboardLost}
          >
            <Dock
              applications={snapshot.applications}
              windows={snapshot.windows}
              activeAppId={snapshot.activeAppId}
              openAtLogin={snapshot.openAtLogin}
              trash={snapshot.trash}
              actions={actions}
              view={view}
              activateApp={pressDockApp}
              showAllWindows={showAllWindows}
              scrollIcon={platform.ScrollTarget ? scrollIcon : undefined}
              ScrollTarget={platform.ScrollTarget}
              toggleLauncher={toggleLauncher}
              toggleOverview={toggleOverview}
              keys={dockKeys && !sheet ? keyboard : undefined}
              moveFocus={moveDockFocus}
              leave={leaveDock}
              MinimizeTarget={platform.MinimizeTarget}
              useDisplayCovered={platform.useDisplayCovered}
              announce={platform.announce}
            />
          </Surface>
        )}
        {/* The lock screen: a surface of its own, all the compositor shows while locked. The
            desktop stays as it was beneath, as it is when the session unlocks. */}
        {lock?.locked && (
          <Surface kind="lock" style={WHOLE}>
            <LockScreen
              lock={lock}
              wallpaper={wallpaper}
              now={now}
              Masked={platform.Masked}
              notifications={notifications}
              applications={snapshot.applications}
              announce={platform.announce}
              safeMode={snapshot.safeMode}
            />
            {/* Locked, the keys still act, and their bezel shows on the lock screen. */}
            <Bezel bezel={controls?.bezel} levels={shownControls.levels} />
          </Surface>
        )}
        {/* A system sheet (ADR 0030): over everything but the lock screen, the session's question
            to the person; only the shell's press answers it. It is modal, and has the keyboard as
            a Mac's alert does: nothing typed while it asks reaches the application behind it. */}
        {/* An application that quit (docs/adr/0061) starts on Reopen, its default button. */}
        {sheet && !lock?.locked && (
          <Surface
            kind="overlay"
            style={WHOLE}
            keyboard
            focusTarget={sheet.text.kind === "crash" && sheet.text.show === "notice" ? "system-sheet-accept" : undefined}
          >
            <SystemSheet
              key={sheet.id}
              text={sheet.text}
              names={sheet.names}
              answer={sheet.answer}
              answerPassword={sheet.answerPassword}
              answerCode={sheet.answerCode}
              answerPasskey={sheet.answerPasskey}
              act={sheet.act}
              copy={sheet.copy}
              keyboard={platform.keyboard}
            />
          </Surface>
        )}
        {/* Screenshots (ADR 0024, decision 10): the crosshair, the window picking and the toolbar
            the compositor opened, and the thumbnail of one saved; never while locked. */}
        {screenshots && !lock?.locked && (
          <Screenshots screenshots={screenshots} platform={platform} dock={dock} dockHeight={DOCK_HEIGHT} />
        )}
        {/* A hardware key's bezel, above everything, taking no input. */}
        {controls?.bezel && !lock?.locked && (
          <Surface kind="notification" style={WHOLE}>
            <Bezel bezel={controls.bezel} levels={shownControls.levels} />
          </Surface>
        )}
      </Box>
    </BloomScope>
  );
});
