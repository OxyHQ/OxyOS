import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Image, useColorScheme, type PointerEvent, type View } from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";
import { Button } from "@oxy.so/bloom/button";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@oxy.so/bloom/context-menu";
import { Tooltip, TooltipTextBubble, TooltipTrigger } from "@oxy.so/bloom/tooltip";
import { DropZone, type DropTargetDropEvent } from "@oxy.so/bloom/drop-zone";
import { RiLayoutGridLine } from "@oxy.so/bloom/icons/RiLayoutGridLine";
import { Box, Scroll } from "../primitives";
import type { MinimizeTargetProps, ScrollTargetProps, ShellKeyboard, ShellScrollEvent } from "../platform";
import { CORNER, IconBody } from "../apps";
import type { ShellActions, ShellApplication, ShellSnapshot, ShellWindow } from "../model";
import { OxyLogo } from "./OxyLogo";
import { TRASH_ICON } from "./trash-icon";
import { belongsTo } from "../model";
import {
  dockMenu,
  iconScroll,
  keepInDock,
  NO_ICON_SCROLL,
  openAtLogin,
  trashMenu,
  trashedByDrop,
  windowIdsByApp,
  windowIdsKey,
  type DockAbilities,
  type DockCommand,
  type DockMenuItem,
} from "../dock";
import { useShellMessages } from "../useShellMessages";

/** The shelf's height: the work area ends above it, and the launcher's overlay with it. */
export const DOCK_HEIGHT = 56;

/** An icon's side, and the shelf's row it sits in. */
const ICON = 42;
const ROW = 55;
/** How far above its icon a menu opens, as a Mac's Dock leaves a gap. */
const MENU_GAP = 10;
/** The Trash's place among the dock's menus. */
const TRASH = "trash";

/** The shelf's top corners with no window covering the display: half its height, as they were
    before 3 November 2026 (ChromeOS's shelf), the wallpaper showing past their curve. */
const SHELF_RADIUS = DOCK_HEIGHT / 2;
/** The zoom's spring (docs/adr/0011-transitions.mdx, step 6: a response of 340 ms, critically
    damped), on which the compositor carries a window's own corners as it fills the work area:
    the shelf's go square with the window's and come back round with them, at rest within a
    thousandth of their way (500 ms), never past it. With Reduce Motion it is at once
    (Reanimated's `ReduceMotion.System`, the session's preference). */
const ZOOM_OMEGA = (2 * Math.PI) / 0.34;
const CORNER_SPRING = {
  mass: 1,
  stiffness: ZOOM_OMEGA * ZOOM_OMEGA,
  damping: 2 * ZOOM_OMEGA,
  overshootClamping: true,
  energyThreshold: 1e-6,
} as const;
const SHELF_LAYOUT = {
  flexShrink: 0,
  alignItems: "center",
  justifyContent: "center",
  overflow: "hidden",
  height: DOCK_HEIGHT,
  zIndex: 40,
} as const;
const neverCovered = () => false;

/** The shelf's colours in the system's appearance (docs/adr/0019-settings.mdx), as a Mac's dock
    turns dark with it. */
const DOCK_COLORS = {
  light: {
    shelf: "#f7f9fc",
    launcher: "#dfe6ed",
    launcherIcon: "#5b6871",
    overview: "#e7edf3",
    overviewIcon: "#56616a",
    running: "#64717a",
    separator: "#d5dbe2",
  },
  dark: {
    shelf: "#1d1f24",
    launcher: "#2c3038",
    launcherIcon: "#c3cad3",
    overview: "#2a2e35",
    overviewIcon: "#c3cad3",
    running: "#aab3bd",
    separator: "#3a3e46",
  },
} as const;

/** A menu open on an icon (an application's id, or the Trash): where it opens (the top of the
    icon's middle, in the window), and whether Option was held as it opened. */
type OpenMenu = { id: string; anchor: { x: number; y: number }; option: boolean };

/** The menu's entries as Bloom's context menu rows, a check where an entry has one. */
function MenuRows({
  items,
  run,
}: {
  items: readonly DockMenuItem[];
  run: (command: DockCommand) => void;
}) {
  return (
    <>
      {items.map((item) => {
        switch (item.kind) {
          case "separator":
            // A separator takes no props, its key its fragment's.
            return (
              <React.Fragment key={item.id}>
                <ContextMenuSeparator />
              </React.Fragment>
            );
          case "label":
            return <ContextMenuLabel key={item.id}>{item.label}</ContextMenuLabel>;
          case "submenu":
            return (
              <ContextMenuSub key={item.id}>
                <ContextMenuSubTrigger>{item.label}</ContextMenuSubTrigger>
                <ContextMenuSubContent label={item.label}>
                  <MenuRows items={item.items} run={run} />
                </ContextMenuSubContent>
              </ContextMenuSub>
            );
          case "item":
            return item.checked !== undefined ? (
              <ContextMenuCheckboxItem
                key={item.id}
                checked={item.checked}
                disabled={item.enabled === false}
                onCheckedChange={() => run(item.command)}
              >
                {item.label}
              </ContextMenuCheckboxItem>
            ) : (
              <ContextMenuItem key={item.id} disabled={item.enabled === false} onPress={() => run(item.command)}>
                {item.label}
              </ContextMenuItem>
            );
        }
      })}
    </>
  );
}

const ICON_ROW = { height: ROW, width: ICON } as const;
const ICON_BOX = { height: ICON, width: ICON } as const;
const SCROLL_STYLE = { flexGrow: 0, maxWidth: "100%" } as const;

/**
 * One icon of the dock, an application's or the Trash, as a Mac's Dock has it
 * (docs/adr/0034-the-dock.mdx):
 * - a press opens it;
 * - the pointer over it shows its name above it, Bloom's tooltip in the platform's popup (the dock
 *   is a strip as tall as itself, which clips anything drawn above it in it), gone as the pointer
 *   leaves and while its menu is open;
 * - a long press (half a second, still), a secondary click or a Control-click opens its menu above
 *   it, Bloom's context menu in the platform's popup, driven from the keyboard and announced as a
 *   menu named by the icon; Option held as it opens makes Quit Force Quit.
 * - an application's picture is where its windows minimize into and are restored out of, where the
 *   platform says so (its MinimizeTarget, docs/adr/0011-transitions.mdx: the genie).
 * Memoized with handlers that stay the same: the pointer moving over the dock, or a window changing
 * size, renders none of them again, and hovering one renders it alone; a window opening or closing
 * renders its application's alone.
 */
const DockIcon = memo(function DockIcon({
  id,
  name,
  testID,
  label,
  status,
  menu,
  setMenu,
  activate,
  runningColor,
  rows,
  app,
  register,
  onFocusIcon,
  windowIds,
  MinimizeTarget,
}: {
  id: string;
  name: string;
  testID: string;
  /** Its name, as the screen reader says it: the application's, as VoiceOver names a Dock item. */
  label: string;
  /** What it is said to be after its name, as VoiceOver says a Dock item running: "en ejecución";
      the Trash "vacía" or "llena". */
  status?: string;
  /** This icon's menu, while it is the one open. */
  menu: OpenMenu | null;
  setMenu: (menu: OpenMenu | null) => void;
  activate: (id: string) => void;
  /** The running mark's colour, under an application running; none for one that is not. */
  runningColor?: string;
  /** Its menu's rows while it is open. */
  rows: React.ReactNode;
  /** The application whose artwork it shows; none, the Trash's bin. */
  app?: ShellApplication;
  /** Hands the dock what opens this icon's menu from the keyboard (↑, as a Mac's Dock). */
  register?: (id: string, open: (() => void) | null) => void;
  /** The keyboard's focus reached the icon. */
  onFocusIcon?: (testID: string) => void;
  /** Its application's windows' ids, one a line (`windowIdsKey`'s): a string, the same from one
      render to the next while the same windows are open. */
  windowIds?: string;
  /** Where the platform minimizes windows into a view; an application's picture is one. */
  MinimizeTarget?: React.ComponentType<MinimizeTargetProps>;
}) {
  // What the screen reader says of it after its name and state, as VoiceOver gives a Dock item its
  // help: the key that opens its menu with the keyboard on the dock (Super+F3, then ↑).
  const menuHint = useShellMessages().dock.menuHint;
  const box = useRef<React.ComponentRef<typeof View>>(null);
  const [hovered, setHovered] = useState(false);
  // Whether Option was held at the press that may open the menu, and whether it was a
  // Control-click (which opens the menu and opens nothing else).
  const option = useRef(false);
  const controlClick = useRef(false);
  const openMenu = useCallback(
    (withOption: boolean) =>
      box.current?.measureInWindow((x, y, width, height) =>
        setMenu({ id, anchor: { x: x + width / 2, y: y + (height - ICON) / 2 }, option: withOption }),
      ),
    [id, setMenu],
  );
  // Heard before the trigger's own (which takes a secondary press for itself): the modifiers held,
  // and a Control-click, a secondary click by another name.
  const onPointerDownCapture = useCallback(
    (event: PointerEvent) => {
      const { button, ctrlKey, altKey } = event.nativeEvent;
      option.current = altKey;
      controlClick.current = button === 0 && ctrlKey;
      if (controlClick.current) openMenu(altKey);
    },
    [openMenu],
  );
  useEffect(() => {
    register?.(id, () => openMenu(false));
    return () => register?.(id, null);
  }, [register, id, openMenu]);
  const onFocus = useCallback(() => onFocusIcon?.(testID), [onFocusIcon, testID]);
  const onAnchorChange = useCallback(
    (anchor: unknown) => {
      if (anchor) openMenu(option.current);
      else setMenu(null);
    },
    [openMenu, setMenu],
  );
  const press = useCallback(() => {
    if (controlClick.current) {
      controlClick.current = false;
      return;
    }
    activate(id);
  }, [activate, id]);
  const hoverIn = useCallback(() => setHovered(true), []);
  const hoverOut = useCallback(() => setHovered(false), []);
  // A press hides the name until the pointer leaves and comes back, as a desktop's tooltip.
  const pressIn = useCallback(() => setHovered(false), []);
  const minimizing = useMemo(() => (windowIds ? windowIds.split("\n") : []), [windowIds]);
  // The icon's button, made again only when what it shows or does changes: the tooltip showing or
  // going renders the tooltip, not the button (whose own hover is Bloom's).
  const trigger = useMemo(
    () => (
      <ContextMenuTrigger asChild>
        <Button
          testID={testID}
          accessibilityLabel={label}
          accessibilityValue={status ? { text: status } : undefined}
          accessibilityHint={menuHint}
          appearance="plain"
          onPress={press}
          onPressIn={pressIn}
          onHoverIn={hoverIn}
          onHoverOut={hoverOut}
          onFocus={onFocus}
          className="h-[42px] w-[42px] overflow-visible rounded-[10px] border-0 bg-transparent p-0"
        >
          {/* Its icon as it is, a Mac's Dock's: no ring or plate around it. An application's,
              even with no window yet, is where its windows minimize into: its own box, 42 × 42,
              not the taller row. */}
          {app && MinimizeTarget ? (
            <MinimizeTarget windowIds={minimizing} style={ICON_BOX}>
              <IconBody icon={app.icon} body={app.iconBody} size={ICON} />
            </MinimizeTarget>
          ) : (
            <IconBody icon={app ? app.icon : TRASH_ICON} body={app?.iconBody} size={ICON} />
          )}
        </Button>
      </ContextMenuTrigger>
    ),
    [testID, label, status, menuHint, press, pressIn, hoverIn, hoverOut, onFocus, app, MinimizeTarget, minimizing],
  );
  // Its menu's panel, made again only as the menu opens, closes or changes.
  const content = useMemo(
    () => (
      <ContextMenuContent side="top" align="center" sideOffset={MENU_GAP} label={name} testID={`${testID}-menu`}>
        {menu ? rows : null}
      </ContextMenuContent>
    ),
    [menu, rows, name, testID],
  );
  return (
    <Box ref={box} className="items-center justify-center" style={ICON_ROW} onPointerDownCapture={onPointerDownCapture}>
      <ContextMenu anchor={menu ? menu.anchor : null} onAnchorChange={onAnchorChange}>
        <Tooltip visible={hovered && !menu} onVisibleChange={setHovered} position="top">
          <TooltipTrigger>{trigger}</TooltipTrigger>
          <TooltipTextBubble>{name}</TooltipTextBubble>
        </Tooltip>
        {content}
      </ContextMenu>
      {runningColor && (
        <Box
          pointerEvents="none"
          testID={`${testID}-running`}
          className="absolute bottom-[2px] h-0.5 w-2.5 rounded-full"
          style={{ backgroundColor: runningColor }}
        />
      )}
    </Box>
  );
});

/**
 * An application's dock icon taking the scrolls over it (docs/adr/0044-app-expose.mdx), where the
 * platform hands them (its ScrollTarget): up opens its App Exposé, down closes it, once a touchpad's
 * scroll (dock.ts, `iconScroll`). Apart from the icon, so the pointer over the icon (its own state)
 * renders the icon alone.
 */
const IconScroll = memo(function IconScroll({
  id,
  scroll,
  ScrollTarget,
  children,
}: {
  id: string;
  scroll: (id: string, up: boolean) => void;
  ScrollTarget: React.ComponentType<ScrollTargetProps>;
  children: React.ReactNode;
}) {
  const scrolled = useRef(NO_ICON_SCROLL);
  const onScroll = useCallback(
    (event: ShellScrollEvent) => {
      const next = iconScroll(scrolled.current, event);
      scrolled.current = next.state;
      if (next.direction) scroll(id, next.direction === "up");
    },
    [scroll, id],
  );
  return (
    <ScrollTarget onScroll={onScroll} style={ICON_ROW}>
      {children}
    </ScrollTarget>
  );
});

const TRASH_ACCEPT = ["files"] as const;
const TRASH_ACTIONS = ["move"] as const;
/** How much the Trash grows while a drag it would take is over it (docs/adr/0044-system-sounds.mdx):
    a little, as the owner asked ("un poco de scale"), the way a Mac's Dock answers a drag. */
export const TRASH_TARGET_SCALE = 1.1;
/** How dark the Trash's own picture goes meanwhile, as a Mac's Dock darkens the icon it would drop
    on. */
export const TRASH_TARGET_SHADE = 0.35;
/** The spring it grows and settles back on: a response of 250 ms, critically damped and never past
    its end, so it only ever goes toward where it is going. */
const TRASH_OMEGA = (2 * Math.PI) / 0.25;
const TRASH_SPRING = {
  mass: 1,
  stiffness: TRASH_OMEGA * TRASH_OMEGA,
  damping: 2 * TRASH_OMEGA,
  overshootClamping: true,
  energyThreshold: 1e-6,
} as const;
/** The shade comes and goes faster than the growth: the answer to the drag is at once. With Reduce
    Motion it is at once (Reanimated's `ReduceMotion.System`). */
const TRASH_SHADE_TIMING = { duration: 120 } as const;
const TRASH_BOX = { height: ROW, width: ICON } as const;
const TRASH_SHADE_LAYOUT = {
  position: "absolute",
  left: 0,
  top: (ROW - ICON) / 2,
  width: ICON,
  height: ICON,
} as const;
/** The picture as the icon draws it (`IconBody`: its corners rounded as every picture's), tinted black. */
const TRASH_SHADE_IMAGE = { width: ICON, height: ICON, borderRadius: ICON * CORNER, tintColor: "#000000" } as const;

/**
 * The Trash as a drop target (docs/adr/0035-drag-and-drop.mdx): files dragged from any
 * application onto it, with a move alone (a Mac's Trash), are put in it by the platform, which
 * plays the Trash's sound (docs/adr/0044-system-sounds.mdx). While a drag it would take is over it,
 * it answers as a Mac's Dock does: its picture darkens, its own shape and nothing around it, and it
 * grows a little on a spring, settling back as the drag leaves or drops. With Reduce Motion it
 * darkens alone, at once, and never grows. The pointer keeps the arrow: a move has no badge.
 */
const TrashDrop = memo(function TrashDrop({
  trashFiles,
  children,
}: {
  trashFiles: (uris: readonly string[]) => void;
  children: React.ReactNode;
}) {
  const reduceMotion = useReducedMotion();
  const scale = useSharedValue(1);
  const shade = useSharedValue(0);
  const target = useCallback(
    (over: boolean) => {
      scale.value = withSpring(over && !reduceMotion ? TRASH_TARGET_SCALE : 1, TRASH_SPRING);
      shade.value = withTiming(over ? TRASH_TARGET_SHADE : 0, TRASH_SHADE_TIMING);
    },
    [scale, shade, reduceMotion],
  );
  const enter = useCallback(() => target(true), [target]);
  const leave = useCallback(() => target(false), [target]);
  const drop = useCallback(
    ({ data }: DropTargetDropEvent) => {
      target(false);
      const uris = trashedByDrop(data.files);
      if (uris.length > 0) trashFiles(uris);
    },
    [trashFiles, target],
  );
  const grown = useAnimatedStyle(() => ({ transform: [{ scale: scale.value }] }));
  const shaded = useAnimatedStyle(() => ({ opacity: shade.value }));
  return (
    <DropZone testID="dock-trash-drop" accept={TRASH_ACCEPT} actions={TRASH_ACTIONS} onDragEnter={enter} onDragLeave={leave} onDrop={drop}>
      <Animated.View testID="dock-trash-scale" style={[TRASH_BOX, grown]}>
        {children}
        <Animated.View testID="dock-trash-shade" pointerEvents="none" style={[TRASH_SHADE_LAYOUT, shaded]}>
          <Image source={TRASH_ICON} resizeMode="cover" style={TRASH_SHADE_IMAGE} />
        </Animated.View>
      </Animated.View>
    </DropZone>
  );
});

/**
 * The shelf itself: one surface from edge to edge of the display, flush with its sides and its
 * bottom, with no outline (docs/adr/0034-the-dock.mdx). Its top corners are round while nothing
 * covers the display, the wallpaper showing past their curve, and square while a window zoomed to
 * the work area or full screen does, so the window meets it along a straight edge, as the window's
 * own corners are square then (docs/adr/0016-windows.mdx, decision 7); between the two they move
 * on the zoom's spring. It alone reads whether the display is covered: a window zoomed or restored
 * renders it and nothing else, its icons and buttons (`children`, made by the dock) kept as they
 * were. A border is never added: on a rounded view it would keep the background 0.8 px inside
 * every edge, as Android's (BackgroundDrawable's pathAdjustment), the wallpaper showing down the
 * display's sides and along its bottom.
 */
const DockShelf = memo(function DockShelf({
  color,
  useDisplayCovered = neverCovered,
  children,
}: {
  color: string;
  useDisplayCovered?: () => boolean;
  children: React.ReactNode;
}) {
  const covered = useDisplayCovered();
  const radius = useSharedValue(covered ? 0 : SHELF_RADIUS);
  useEffect(() => {
    radius.value = withSpring(covered ? 0 : SHELF_RADIUS, CORNER_SPRING);
  }, [covered, radius]);
  const corners = useAnimatedStyle(() => ({
    borderTopLeftRadius: radius.value,
    borderTopRightRadius: radius.value,
  }));
  const style = useMemo(() => [SHELF_LAYOUT, { backgroundColor: color }, corners], [color, corners]);
  // The same element while its style and children are: the shelf's view renders again only when
  // its colour or its contents change, never for its corners, which Reanimated moves.
  return useMemo(
    () => (
      <Animated.View testID="shell-dock" style={style}>
        {children}
      </Animated.View>
    ),
    [style, children],
  );
});

/** The applications with a window, in the registry's order, as one string: the same while the
    same applications run, whatever else changes of their windows (a size, a title). */
function runningKey(applications: readonly ShellApplication[], windows: readonly ShellWindow[]): string {
  return applications
    .filter((app) => windows.some((win) => belongsTo(win, app.id)))
    .map((app) => app.id)
    .join("\n");
}

export const Dock = memo(function Dock({
  applications,
  windows,
  activeAppId,
  openAtLogin: atLogin,
  trash,
  actions,
  view,
  activateApp,
  showAllWindows,
  toggleLauncher,
  toggleOverview,
  keys,
  moveFocus,
  leave,
  MinimizeTarget,
  useDisplayCovered,
  scrollIcon,
  ScrollTarget,
}: {
  applications: readonly ShellApplication[];
  windows: readonly ShellWindow[];
  activeAppId?: string;
  openAtLogin?: readonly string[];
  /** The Trash, where the platform has one (docs/adr/0034-the-dock.mdx). */
  trash?: ShellSnapshot["trash"];
  /** What the platform does for the dock's menus: the same object from one render to the next. */
  actions: ShellActions;
  view: string;
  activateApp: (id: string) => void;
  /** The application's App Exposé (docs/adr/0044-app-expose.mdx): its windows alone, large. */
  showAllWindows?: (id: string) => void;
  /** A scroll up (or down) on an application's icon: its App Exposé opens (or closes), as a
      Mac's Dock can; and where the platform hands an icon the scrolls over it. */
  scrollIcon?: (id: string, up: boolean) => void;
  ScrollTarget?: React.ComponentType<ScrollTargetProps>;
  toggleLauncher: () => void;
  toggleOverview: () => void;
  /** The dock's keys while it has the keyboard (Super+F3, docs/adr/0039-keyboard-access.mdx): ← →
      go along it, ↑ opens the focused icon's menu, Escape leaves it; Tab, Space and Return are the
      platform's. */
  keys?: ShellKeyboard;
  /** Puts the keyboard's focus on the icon of that testID. */
  moveFocus?: (testID: string) => void;
  /** Leaves the dock, the keyboard back with the window. */
  leave?: () => void;
  /** Where the platform minimizes windows into a view (the platform's MinimizeTarget): each
      application's icon, for its windows. */
  MinimizeTarget?: React.ComponentType<MinimizeTargetProps>;
  /** Whether a zoomed or full-screen window covers this display (the platform's): the shelf's
      top corners are square while one does. */
  useDisplayCovered?: () => boolean;
}) {
  const colors = DOCK_COLORS[useColorScheme() === "dark" ? "dark" : "light"];
  const messages = useShellMessages();
  const words = messages.dock;
  const [menu, setMenu] = useState<OpenMenu | null>(null);
  // What the shelf shows changes only with the applications that run: a window resized, moved or
  // retitled leaves it as it was.
  const runningIds = runningKey(applications, windows);
  const running = useMemo(() => new Set(runningIds ? runningIds.split("\n") : []), [runningIds]);
  const apps = useMemo(
    () => applications.filter((app) => app.pinned || running.has(app.id)),
    [applications, running],
  );
  // Each icon's windows, where the platform minimizes into icons: changed only as windows open or
  // close, and then only for their application's icon.
  const windowsKey = MinimizeTarget ? windowIdsKey(apps, windows) : "";
  const windowIds = useMemo(() => windowIdsByApp(windowsKey), [windowsKey]);
  const abilities = useMemo<DockAbilities>(
    () => ({
      hide: !!actions.hideApp,
      quit: !!actions.quitApp,
      forceQuit: !!actions.forceQuitApp,
      keepInDock: !!actions.setDock,
      openAtLogin: !!actions.setOpenAtLogin,
      showAllWindows: !!showAllWindows,
    }),
    [actions, showAllWindows],
  );
  // A command acts on what is shown when it is chosen.
  const latest = useRef({ applications, windows, activeAppId, openAtLogin: atLogin });
  latest.current = { applications, windows, activeAppId, openAtLogin: atLogin };
  const run = useCallback(
    (id: string, command: DockCommand) => {
      setMenu(null);
      const state = latest.current;
      switch (command.kind) {
        case "window":
          return actions.focusWindow(command.window);
        case "open":
          return activateApp(id);
        case "show-all-windows":
          return showAllWindows?.(id);
        case "hide":
          return actions.hideApp?.(id);
        case "keep-in-dock":
          return actions.setDock?.(keepInDock(state, id, command.keep));
        case "open-at-login":
          return actions.setOpenAtLogin?.(openAtLogin(state, id, command.open));
        case "quit":
          return actions.quitApp?.(id);
        case "force-quit":
          return actions.forceQuitApp?.(id);
      }
    },
    [actions, activateApp, showAllWindows],
  );
  // The open menu's rows, worked out only while one is open: nothing that changes while it is
  // closed renders an icon again.
  const rows = useMemo(() => {
    if (!menu) return null;
    if (menu.id === TRASH) {
      return trashMenu(trash?.items, !!actions.emptyTrash, messages).map((entry) => (
        <ContextMenuItem
          key={entry.id}
          disabled={!entry.enabled}
          onPress={() => {
            setMenu(null);
            if (entry.id === "open") actions.openTrash?.();
            else actions.emptyTrash?.();
          }}
        >
          {entry.label}
        </ContextMenuItem>
      ));
    }
    const items = dockMenu(
      { applications, windows, activeAppId, openAtLogin: atLogin },
      menu.id,
      abilities,
      messages,
      menu.option,
    );
    return <MenuRows items={items} run={(command) => run(menu.id, command)} />;
  }, [menu, applications, windows, activeAppId, atLogin, abilities, messages, trash, actions, run]);
  const openTrash = useCallback(() => actions.openTrash?.(), [actions]);
  // Whether the Trash holds anything, as a Mac's Trash is said empty or full (when the platform says).
  const trashStatus = trash?.items === undefined ? undefined : trash.items > 0 ? words.trashFull : words.trashEmpty;
  // The keyboard along the dock: the icon its focus is on, and each icon's menu opener.
  const focusedIcon = useRef<string | null>(null);
  const onFocusIcon = useCallback((testID: string) => {
    focusedIcon.current = testID;
  }, []);
  const openers = useRef(new Map<string, () => void>());
  const register = useCallback((id: string, open: (() => void) | null) => {
    if (open) openers.current.set(id, open);
    else openers.current.delete(id);
  }, []);
  const order = useMemo(
    () => ["dock-launcher", "dock-overview", ...apps.map((app) => `dock-${app.id}`), ...(trash ? ["dock-trash"] : [])],
    [apps, trash],
  );
  const latestOrder = useRef(order);
  latestOrder.current = order;
  useEffect(() => {
    // An open menu's keys are its own.
    if (!keys || menu) return undefined;
    return keys.subscribe((event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      const all = latestOrder.current;
      const at = focusedIcon.current ? all.indexOf(focusedIcon.current) : -1;
      switch (event.key) {
        case "Escape":
          leave?.();
          return true;
        case "ArrowLeft":
        case "ArrowRight": {
          const step = event.key === "ArrowRight" ? 1 : -1;
          const next = all[Math.min(all.length - 1, Math.max(0, at + step))];
          if (next) moveFocus?.(next);
          return true;
        }
        case "ArrowUp": {
          const icon = focusedIcon.current;
          const id = icon === "dock-trash" ? TRASH : icon?.startsWith("dock-") ? icon.slice(5) : null;
          const open = id ? openers.current.get(id) : undefined;
          if (!open) return false;
          open();
          return true;
        }
        default:
          return false;
      }
    });
  }, [keys, menu, moveFocus, leave]);
  const trashFiles = actions.trashFiles;
  const dividerStyle = useMemo(() => ({ width: 1, height: 28, backgroundColor: colors.separator }), [colors]);
  // The shelf, made again only when what it shows changes: a window resized, moved or retitled,
  // or the pointer over an icon (whose own state it is), renders none of it.
  const shelf = useMemo(
    () => (
      <Scroll
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="h-[55px] flex-row items-center justify-center gap-4 px-6"
        style={SCROLL_STYLE}
      >
        <Button
          testID="dock-launcher"
          iconOnly
          icon={OxyLogo}
          accessibilityLabel={words.openApplications}
          appearance="plain"
          onPress={toggleLauncher}
          onFocus={() => onFocusIcon("dock-launcher")}
          pressed={view === "launcher"}
          className="h-10 w-16 rounded-full border-0 p-0"
          iconSize={25}
          colors={{ background: colors.launcher, foreground: colors.launcherIcon }}
        />
        <Button
          testID="dock-overview"
          iconOnly
          icon={RiLayoutGridLine}
          accessibilityLabel={messages.common.showDesktops}
          appearance="plain"
          onPress={toggleOverview}
          onFocus={() => onFocusIcon("dock-overview")}
          pressed={view === "overview"}
          className="h-[42px] w-[42px] rounded-full border-0 p-0"
          iconSize={22}
          colors={{ background: colors.overview, foreground: colors.overviewIcon }}
        />
        {apps.map((app) => {
          const icon = (
            <DockIcon
              key={app.id}
              id={app.id}
              name={app.name}
              testID={`dock-${app.id}`}
              label={app.name}
              status={running.has(app.id) ? words.running : undefined}
              menu={menu?.id === app.id ? menu : null}
              setMenu={setMenu}
              activate={activateApp}
              runningColor={running.has(app.id) ? colors.running : undefined}
              rows={menu?.id === app.id ? rows : null}
              app={app}
              register={register}
              onFocusIcon={onFocusIcon}
              windowIds={windowIds.get(app.id)}
              MinimizeTarget={MinimizeTarget}
            />
          );
          // Its scrolls, where the platform hands them.
          return scrollIcon && ScrollTarget ? (
            <IconScroll key={app.id} id={app.id} scroll={scrollIcon} ScrollTarget={ScrollTarget}>
              {icon}
            </IconScroll>
          ) : (
            icon
          );
        })}
        {trash && (
          <>
            {/* The Trash sits past a divider at the dock's end, as a Mac's. */}
            <Box style={dividerStyle} />
            {trashFiles ? (
              <TrashDrop trashFiles={trashFiles}>
                <DockIcon
                  id={TRASH}
                  name={messages.common.trash}
                  testID="dock-trash"
                  label={messages.common.trash}
                  status={trashStatus}
                  menu={menu?.id === TRASH ? menu : null}
                  setMenu={setMenu}
                  activate={openTrash}
                  rows={menu?.id === TRASH ? rows : null}
                  register={register}
                  onFocusIcon={onFocusIcon}
                />
              </TrashDrop>
            ) : (
              <DockIcon
                id={TRASH}
                name={messages.common.trash}
                testID="dock-trash"
                label={messages.common.trash}
                status={trashStatus}
                menu={menu?.id === TRASH ? menu : null}
                setMenu={setMenu}
                activate={openTrash}
                rows={menu?.id === TRASH ? rows : null}
                register={register}
                onFocusIcon={onFocusIcon}
              />
            )}
          </>
        )}
      </Scroll>
    ),
    [apps, running, colors, messages, view, menu, rows, trash, trashStatus, trashFiles, activateApp, openTrash, toggleLauncher, toggleOverview, dividerStyle, register, onFocusIcon, windowIds, MinimizeTarget, scrollIcon, ScrollTarget],
  );
  return (
    <DockShelf color={colors.shelf} useDisplayCovered={useDisplayCovered}>
      {shelf}
    </DockShelf>
  );
});
