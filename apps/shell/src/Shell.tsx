import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AccessibilityInfo, StyleSheet, useColorScheme, type ImageSourcePropType } from "react-native";
import { BloomScope } from "@oxy.so/bloom/appearance";
import { Box } from "./primitives";
import type {
  ShellActions,
  ShellCommand,
  ShellControls,
  ShellLock,
  ShellNotification,
  ShellNotifications,
  ShellScreenshots,
  ShellSnapshot,
} from "./model";
import { Wallpaper } from "./components/Wallpaper";
import { TopBar } from "./components/TopBar";
import { DOCK_HEIGHT, Dock } from "./components/Dock";
import { Overview } from "./components/Overview";
import { Launchpad, LaunchpadBackdrop } from "./components/Launchpad";
import { LockScreen } from "./components/LockScreen";
import { AppSwitcher } from "./components/AppSwitcher";
import { Bezel } from "./components/Bezel";
import { ControlCenter, type ControlCenterPage } from "./components/ControlCenter";
import { BANNER_WIDTH, NotificationCard } from "./components/NotificationBanner";
import { NotificationCenter } from "./components/NotificationCenter";
import { SystemSheet } from "./components/SystemSheet";
import { Screenshots } from "./components/Screenshots";
import type { Language, SheetText } from "./sheet";
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
import { belongsTo } from "./model";
export type { ShellCommand } from "./model";
export type {
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
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  wallpaper: ImageSourcePropType;
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
    language?: Language;
    names?: Record<string, string>;
    answer: (accepted: boolean) => void;
    /** A Wi-Fi password sheet's answer (docs/adr/0038-networks.mdx): the password, to the
        session alone. Without it such a sheet cannot be answered but by Cancel. */
    answerPassword?: (password: string) => void;
  };
  /** Screenshots, where the platform takes them (docs/adr/0024-system-shortcuts.mdx, decision 10):
      the interface the compositor opened from the keyboard, and the thumbnail of one saved. */
  screenshots?: ShellScreenshots;
}) {
  const { Surface } = platform;
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
  const [replying, setReplying] = useState(false);
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
      setReplying(false);
      // Said politely, after what the screen reader is saying, as VoiceOver says a banner.
      const said = spoken(next, applicationOf(next, snapshot.applications).name);
      if (platform.announce) platform.announce(said, true);
      else AccessibilityInfo.announceForAccessibility(said);
    } else {
      // One no longer kept (opened, removed) goes with its banner.
      setBanner((shown) => (shown && list.some((notification) => notification.id === shown.id) ? shown : null));
    }
  }, [list]);
  // A banner stays five seconds, or while the person is replying in it.
  useEffect(() => {
    if (!banner || replying) return undefined;
    const timer = setTimeout(() => setBanner(null), BANNER_MS);
    return () => clearTimeout(timer);
  }, [banner, replying]);
  const [view, setView] = useState<"desktop" | "launcher" | "overview">(
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
  // The application switcher (docs/adr/0024-system-shortcuts.mdx), open while Super is held.
  const [switcher, setSwitcher] = useState<Switcher | null>(null);
  // Commands act on what is shown now, whichever render they arrive in.
  const latest = useRef({ snapshot, actions, switcher, view, controls, statusKeys });
  latest.current = { snapshot, actions, switcher, view, controls, statusKeys };
  // The application whose windows the overview shows alone (the dock menu's Show All Windows, as
  // a Mac's App Exposé, docs/adr/0034-the-dock.mdx); none, every window.
  const [overviewApp, setOverviewApp] = useState<string | null>(null);
  const dismiss = useCallback(() => {
    setView("desktop");
    setOverviewApp(null);
  }, []);
  const toggleOverview = useCallback(() => {
    setOverviewApp(null);
    setView((current) => (current === "overview" ? "desktop" : "overview"));
  }, []);
  const toggleLauncher = useCallback(() => {
    setOverviewApp(null);
    setView((current) => (current === "launcher" ? "desktop" : "launcher"));
  }, []);
  const showAllWindows = useCallback((id: string) => {
    setOverviewApp(id);
    setView("overview");
  }, []);
  const activateApp = useCallback(
    (id: string) => {
      latest.current.actions.activateApp(id);
      setView("desktop");
    },
    [],
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
    for (const next of pending) {
      switch (next.action) {
        case "dismiss":
          setEscapeRequest((value) => value + 1);
          break;
        case "launcher":
        case "overview":
          view = next.action;
          break;
        // Super+Space: the launcher, its search taking what is typed; again, it closes.
        case "search":
          if (view === "launcher") {
            view = "desktop";
          } else {
            view = "launcher";
            setSearchRequest((value) => value + 1);
          }
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
            target: firstStatusItem(now.controls?.network),
            request: (keys?.request ?? 0) + 1,
          }));
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
    if (view !== latest.current.view) setOverviewApp(null);
    setView(view);
  }, [command, commands]);
  // An application quit while the switcher is open leaves it.
  useEffect(() => {
    setSwitcher((current) => current && followSwitcher(current, snapshot));
  }, [snapshot]);
  // Show All Windows: the overview with that application's windows alone.
  const overviewSnapshot = useMemo(
    () =>
      overviewApp
        ? { ...snapshot, windows: snapshot.windows.filter((win) => belongsTo(win, overviewApp)) }
        : snapshot,
    [snapshot, overviewApp],
  );
  const launcherOpen = view === "launcher";
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
  const menuCommand = useCallback(
    (id: string) => {
      if (id === "view.launcher" || id === "view.overview") setOverviewApp(null);
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
            setCenter((open) => !open);
          }
        : undefined,
    [hasNotifications],
  );
  const closeControlCenter = useCallback(() => setControlCenter(null), []);
  // Moving between its modules and Wi-Fi's view from the keyboard keeps the keyboard in it, on
  // the new page's first control (the control it was on is gone).
  const showControlPage = useCallback((page: ControlCenterPage | null) => {
    setControlKeys((value) => (value > 0 ? value + 1 : value));
    setControlCenter(page);
  }, []);
  useEffect(() => {
    if (!controlCenter) setControlKeys(0);
  }, [controlCenter]);
  // The top bar, the dock and the overlays with the keyboard hear Escape and their own keys
  // (docs/adr/0039-keyboard-access.mdx): an overlay closes, the dock and the status items give
  // the window back the keyboard. A sheet over them takes its own keys.
  const keyboard = platform.keyboard;
  const keys = useRef({ controlCenter, center, view, statusKeys, dockKeys, sheet: !!sheet });
  keys.current = { controlCenter, center, view, statusKeys, dockKeys, sheet: !!sheet };
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      const now = keys.current;
      if (now.sheet || event.key !== "Escape") return false;
      if (now.controlCenter) {
        setControlCenter(null);
        return true;
      }
      if (now.center) {
        setCenter(false);
        return true;
      }
      if (now.view === "overview") {
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
  const overlayOpen = !!controlCenter || center || view === "overview";
  const wasOpen = useRef(overlayOpen);
  useEffect(() => {
    const closed = wasOpen.current && !overlayOpen;
    wasOpen.current = overlayOpen;
    if (closed && !keys.current.statusKeys && !keys.current.sheet && keys.current.view !== "launcher") {
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
    const said = `Conectado a ${joined}`;
    if (platform.announce) platform.announce(said, true);
    else AccessibilityInfo.announceForAccessibility(said);
  }, [joined]);
  const openSettings = useCallback(() => menuCommand("system.settings"), [menuCommand]);
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
          <Wallpaper source={wallpaper} />
        </Box>
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
            keyboardRequest={barRequest}
            keyboardLost={barLost}
            onKeyboard={setBarKeyboard}
            returnKeyboard={actions.returnKeyboard}
            menus={snapshot.menus}
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
            statusKeys={statusKeys && !controlCenter && !center && !sheet ? keyboard : undefined}
            moveStatusFocus={moveStatusFocus}
            leaveStatus={leaveStatus}
          />
        </Surface>
        <Box
          className="min-h-0 flex-1"
          pointerEvents={view === "desktop" ? "box-none" : "auto"}
        >
          {view === "overview" && (
            <Surface kind="overlay" style={WHOLE} keyboard={!sheet}>
              <Overview
                snapshot={overviewSnapshot}
                actions={actions}
                dismiss={dismiss}
                escapeRequest={escapeRequest}
                WindowView={platform.WindowView}
              />
            </Surface>
          )}
        </Box>
        {/* The launcher covers the whole screen, the top bar included, but the dock: as Launchpad
            covers the menu bar and leaves the Dock, whose button closes it again. Its blurred
            backdrop is a surface of its own beneath its icons', so the compositor fades the one
            and settles the other (docs/adr/0011-transitions.mdx). It is built once, when the
            shell starts, and kept while hidden, as macOS keeps Launchpad: opening it shows what
            is built. Hidden, it has no surfaces and takes no input. */}
        <Surface kind="backdrop" style={launcherArea}>
          <LaunchpadBackdrop open={launcherOpen} wallpaper={wallpaper} />
        </Surface>
        <Surface kind="overlay" style={launcherArea}>
          <Launchpad
            open={launcherOpen}
            escapeRequest={escapeRequest}
            searchRequest={searchRequest}
            applications={snapshot.applications}
            folders={snapshot.folders}
            launch={activateApp}
            dismiss={dismiss}
            keyboard={platform.keyboard}
          />
        </Surface>
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
              closeLabel="Ocultar"
              close={closeBanner}
              onReplying={setReplying}
              activate={activateBanner}
            />
          </Surface>
        )}
        {notifications && center && !lock?.locked && (
          <Surface kind="overlay" style={WHOLE} keyboard={!sheet}>
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
              activateApp={activateApp}
              showAllWindows={showAllWindows}
              toggleLauncher={toggleLauncher}
              toggleOverview={toggleOverview}
              keys={dockKeys && !sheet ? keyboard : undefined}
              moveFocus={moveDockFocus}
              leave={leaveDock}
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
            />
            {/* Locked, the keys still act, and their bezel shows on the lock screen. */}
            <Bezel bezel={controls?.bezel} levels={shownControls.levels} />
          </Surface>
        )}
        {/* A system sheet (ADR 0030): over everything but the lock screen, the session's question
            to the person; only the shell's press answers it. It is modal, and has the keyboard as
            a Mac's alert does: nothing typed while it asks reaches the application behind it. */}
        {sheet && !lock?.locked && (
          <Surface kind="overlay" style={WHOLE} keyboard>
            <SystemSheet
              key={sheet.id}
              text={sheet.text}
              language={sheet.language}
              names={sheet.names}
              answer={sheet.answer}
              answerPassword={sheet.answerPassword}
              keyboard={platform.keyboard}
              announce={platform.announce}
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
