import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Image,
  useColorScheme,
  type AccessibilityActionEvent,
  type AccessibilityActionInfo,
  type PointerEvent,
  type View,
} from "react-native";
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
import { DropZone, type DropTargetDragEvent, type DropTargetDropEvent } from "@oxy.so/bloom/drop-zone";
import { usePlatformDrag } from "@oxy.so/bloom/platform-surface";
import { RiLayoutGridLine } from "@oxy.so/bloom/icons/RiLayoutGridLine";
import { Box, Scroll } from "../primitives";
import type { MinimizeTargetProps, ScrollTargetProps, ShellKeyboard, ShellScrollEvent } from "../platform";
import { CORNER, IconBody, isSvgIcon } from "../apps";
import type { ShellActions, ShellApplication, ShellSnapshot, ShellWindow } from "../model";
import { OxyLogo } from "./OxyLogo";
import { TRASH_ICON } from "./trash-icon";
import { belongsTo } from "../model";
import {
  DOCK_SLOT,
  dockMenu,
  gapAt,
  iconScroll,
  keptIds,
  keptWithAt,
  keepInDock,
  moveInDock,
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
import { APPLICATION_TYPE, DRAG_SLOP, shellDrag } from "../app-drag";

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
        return null;
      })}
    </>
  );
}

const ICON_ROW = { height: ROW, width: ICON } as const;
/** The row's padding at either end, the launcher's width and the space between two icons. */
const ROW_PADDING = 24;
const LAUNCHER_WIDTH = 64;
const ICON_GAP = 16;
/** Half the space between two icons: each place has half on either side of its icon, and the
    applications' places begin half a space past the overview's button. */
const HALF_GAP = (DOCK_SLOT - ICON) / 2;
const OVERVIEW_PLACE = { marginLeft: ICON_GAP } as const;
const TRASH_PLACE = { flexDirection: "row", alignItems: "center", marginLeft: HALF_GAP } as const;
const SHELF_DROP = { alignSelf: "stretch", height: DOCK_HEIGHT, alignItems: "center", justifyContent: "center" } as const;
const APP_ACCEPT = [APPLICATION_TYPE] as const;
const APP_ACTIONS = ["move", "copy"] as const;
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
  arrangeActions,
  arrange,
  startDrag,
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
  /** Its press, and whether Option (Alt) was held at it (docs/adr/0034-the-dock.mdx, decision 9). */
  activate: (id: string, option: boolean) => void;
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
  /** What an assistive technology can do to arrange it (docs/adr/0034-the-dock.mdx, decision
      10): move it left or right, keep it or take it out of the dock. The same array while its
      place is the same. */
  arrangeActions?: readonly AccessibilityActionInfo[];
  arrange?: (id: string, action: string) => void;
  /** Its press moved past the slop with the primary button (a mouse's, not Control's): the dock
      drags it, its picture `preview` the drag's image (docs/adr/0034-the-dock.mdx, decision 10). */
  startDrag?: (id: string, preview: React.RefObject<React.ComponentRef<typeof View> | null>) => void;
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
  // Where a primary press began, while it may still become a drag; and the picture dragged.
  const pressedAt = useRef<{ x: number; y: number } | null>(null);
  const picture = useRef<React.ComponentRef<typeof View>>(null);
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
      const { button, ctrlKey, altKey, clientX, clientY } = event.nativeEvent;
      option.current = altKey;
      controlClick.current = button === 0 && ctrlKey;
      pressedAt.current = button === 0 && !ctrlKey && startDrag ? { x: clientX, y: clientY } : null;
      if (controlClick.current) openMenu(altKey);
    },
    [openMenu, startDrag],
  );
  // The press moved past the slop: a drag, which the platform takes from here (its press is
  // cancelled, so it is no click), the picture as it looks its image.
  const onPointerMove = useCallback(
    (event: PointerEvent) => {
      const at = pressedAt.current;
      if (!at || !startDrag) return;
      const { clientX, clientY } = event.nativeEvent;
      if (Math.hypot(clientX - at.x, clientY - at.y) < DRAG_SLOP) return;
      pressedAt.current = null;
      setHovered(false);
      startDrag(id, picture);
    },
    [startDrag, id],
  );
  const onPointerUp = useCallback(() => {
    pressedAt.current = null;
  }, []);
  useEffect(() => {
    register?.(id, () => openMenu(false));
    return () => register?.(id, null);
  }, [register, id, openMenu]);
  const onFocus = useCallback(() => onFocusIcon?.(testID), [onFocusIcon, testID]);
  const onAccessibilityAction = useCallback(
    (event: AccessibilityActionEvent) => arrange?.(id, event.nativeEvent.actionName),
    [arrange, id],
  );
  const onAnchorChange = useCallback(
    (anchor: unknown) => {
      if (anchor) openMenu(option.current);
      else setMenu(null);
    },
    [openMenu, setMenu],
  );
  const press = useCallback(() => {
    // Option counts for the press it was held at alone: a key's press (Space, Return) has none.
    const withOption = option.current;
    option.current = false;
    if (controlClick.current) {
      controlClick.current = false;
      return;
    }
    activate(id, withOption);
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
          accessibilityActions={arrangeActions as AccessibilityActionInfo[] | undefined}
          onAccessibilityAction={arrangeActions ? onAccessibilityAction : undefined}
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
              <Box ref={picture} collapsable={false} style={ICON_BOX}>
                <IconBody icon={app.icon} body={app.iconBody} size={ICON} />
              </Box>
            </MinimizeTarget>
          ) : app ? (
            <Box ref={picture} collapsable={false} style={ICON_BOX}>
              <IconBody icon={app.icon} body={app.iconBody} size={ICON} />
            </Box>
          ) : (
            <IconBody icon={TRASH_ICON} size={ICON} />
          )}
        </Button>
      </ContextMenuTrigger>
    ),
    [testID, label, status, menuHint, press, pressIn, hoverIn, hoverOut, onFocus, app, MinimizeTarget, minimizing, arrangeActions, onAccessibilityAction],
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
    <Box
      ref={box}
      className="items-center justify-center"
      style={ICON_ROW}
      onPointerDownCapture={onPointerDownCapture}
      onPointerMove={startDrag ? onPointerMove : undefined}
      onPointerUp={startDrag ? onPointerUp : undefined}
      onPointerCancel={startDrag ? onPointerUp : undefined}
    >
      <ContextMenu anchor={menu ? menu.anchor : null} onAnchorChange={onAnchorChange}>
        {/* The icon's name shows at once, as a Mac's Dock shows it: a desktop's one-second help-tag
          delay (Bloom's default there) is for tags that explain, not for a name. */}
        <Tooltip visible={hovered && !menu} onVisibleChange={setHovered} position="top" delay={0}>
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
 * An application's dock icon taking the scrolls over it (docs/adr/0045-app-expose.mdx), where the
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

/** The spring an application's place opens and closes on as icons are dragged along the dock, a
    Mac's Dock's sliding apart: a response of 250 ms, critically damped and never past its end. */
const SLOT_OMEGA = (2 * Math.PI) / 0.25;
const SLOT_SPRING = {
  mass: 1,
  stiffness: SLOT_OMEGA * SLOT_OMEGA,
  damping: 2 * SLOT_OMEGA,
  overshootClamping: true,
  energyThreshold: 1e-6,
} as const;
/** An application's place: its icon with half the space to each neighbour, clipped as it closes. */
const SLOT_LAYOUT = { height: ROW, overflow: "hidden", alignItems: "center" } as const;
const APPS_LAYOUT = { flexDirection: "row", alignItems: "center", marginLeft: HALF_GAP } as const;

/**
 * An application's place in the dock, or the gap a drag opens between two (docs/adr/0034-the-dock.mdx,
 * decision 10): `open`, it is a place wide; closed, nothing. It opens and closes on a spring, the
 * icons beside it sliding apart and together as a Mac's Dock's do, or at once where `jump` changed
 * (the dock taking the order the person dropped: everything already where it goes) and with Reduce
 * Motion.
 */
const DockSlot = memo(function DockSlot({
  open,
  jump,
  testID,
  children,
}: {
  open: boolean;
  jump: number;
  testID?: string;
  children?: React.ReactNode;
}) {
  const reduceMotion = useReducedMotion();
  const width = useSharedValue(open ? DOCK_SLOT : 0);
  const jumped = useRef(jump);
  useEffect(() => {
    const target = open ? DOCK_SLOT : 0;
    const now = jumped.current !== jump;
    jumped.current = jump;
    width.value = now || reduceMotion ? target : withSpring(target, SLOT_SPRING);
  }, [open, jump, reduceMotion, width]);
  const sized = useAnimatedStyle(() => ({ width: width.value }));
  return (
    <Animated.View testID={testID} style={[SLOT_LAYOUT, sized]}>
      {children}
    </Animated.View>
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

const FILES_ACCEPT = ["files"] as const;
const OPEN_ACTIONS = ["copy"] as const;
const APP_SHADE_LAYOUT = TRASH_SHADE_LAYOUT;
const APP_SHADE_IMAGE = TRASH_SHADE_IMAGE;
const APP_SHADE_BODY = { width: ICON, height: ICON, borderRadius: ICON * CORNER, backgroundColor: "#000000" } as const;

/**
 * An application's icon as a drop target (docs/adr/0034-the-dock.mdx, decision 11): files dropped on
 * it open in it, as on a Mac. It takes a drag only when the application declares it opens every
 * dragged file's type (its entry's MimeType, `opens`); then its picture darkens as the Trash's does
 * (its own shape, in 120 ms, at once with Reduce Motion), and nothing else answers. The session
 * opens the files with it, starting it if it is not running.
 */
const AppDrop = memo(function AppDrop({
  app,
  openFiles,
  children,
}: {
  app: ShellApplication;
  openFiles: (id: string, uris: readonly string[]) => void;
  children: React.ReactNode;
}) {
  const shade = useSharedValue(0);
  const enter = useCallback(() => {
    shade.value = withTiming(TRASH_TARGET_SHADE, TRASH_SHADE_TIMING);
  }, [shade]);
  const leave = useCallback(() => {
    shade.value = withTiming(0, TRASH_SHADE_TIMING);
  }, [shade]);
  const drop = useCallback(
    ({ data }: DropTargetDropEvent) => {
      leave();
      const uris = trashedByDrop(data.files);
      if (uris.length > 0) openFiles(app.id, uris);
    },
    [openFiles, app.id, leave],
  );
  const shaded = useAnimatedStyle(() => ({ opacity: shade.value }));
  const opens = app.opens ?? NO_TYPES;
  return (
    <DropZone
      testID={`dock-drop-${app.id}`}
      accept={FILES_ACCEPT}
      acceptFileTypes={opens}
      disabled={opens.length === 0}
      actions={OPEN_ACTIONS}
      onDragEnter={enter}
      onDragLeave={leave}
      onDrop={drop}
    >
      <Box style={TRASH_BOX}>
        {children}
        <Animated.View testID={`dock-shade-${app.id}`} pointerEvents="none" style={[APP_SHADE_LAYOUT, shaded]}>
          {isSvgIcon(app.icon) ? (
            <Box style={APP_SHADE_BODY} />
          ) : (
            <Image source={app.icon} resizeMode="cover" style={APP_SHADE_IMAGE} />
          )}
        </Animated.View>
      </Box>
    </DropZone>
  );
});
const NO_TYPES: readonly string[] = [];

/**
 * The dock's applications in their places (docs/adr/0034-the-dock.mdx, decision 10): each in its
 * place; the gap a drag opens before the one it would go before, or after those kept; the dragged
 * icon's own place closed while it is off it. Apart from the rest of the shelf, so that what the
 * launcher's and the overview's buttons show (Launchpad, Mission Control or App Exposé open) renders
 * none of it.
 */
const DockApps = memo(function DockApps({
  apps,
  kept,
  drag,
  jump,
  running,
  runningWord,
  runningColor,
  menu,
  rows,
  setMenu,
  activateApp,
  register,
  onFocusIcon,
  windowIds,
  MinimizeTarget,
  arrangeActions,
  arrange,
  startDrag,
  scrollIcon,
  ScrollTarget,
  openFiles,
}: {
  apps: readonly ShellApplication[];
  kept: readonly string[];
  drag: { id: string | null; fromDock: boolean; gap: number | null } | null;
  jump: number;
  running: ReadonlySet<string>;
  runningWord: string;
  runningColor: string;
  menu: OpenMenu | null;
  rows: React.ReactNode;
  setMenu: (menu: OpenMenu | null) => void;
  activateApp: (id: string, option: boolean) => void;
  register: (id: string, open: (() => void) | null) => void;
  onFocusIcon: (testID: string) => void;
  windowIds: ReadonlyMap<string, string>;
  MinimizeTarget?: React.ComponentType<MinimizeTargetProps>;
  arrangeActions: ReadonlyMap<string, AccessibilityActionInfo[]>;
  arrange: (id: string, action: string) => void;
  startDrag?: (id: string, preview: React.RefObject<React.ComponentRef<typeof View> | null>) => void;
  scrollIcon?: (id: string, up: boolean) => void;
  ScrollTarget?: React.ComponentType<ScrollTargetProps>;
  openFiles?: (id: string, uris: readonly string[]) => void;
}) {
  return (
    <Box style={APPS_LAYOUT}>
      {(() => {
        // Each application in its place; the gap a drag opens before the one it would go
        // before, or after those kept; the dragged icon's own place closed while it is off it.
        const draggedKept = drag?.id && kept.includes(drag.id) ? drag.id : null;
        const out = draggedKept !== null && (drag?.fromDock || drag?.gap !== null);
        const others = kept.filter((id) => id !== draggedKept);
        const gapAfter = (index: number) => drag !== null && drag.gap === index;
        const nodes: React.ReactNode[] = [];
        for (const app of apps) {
          const keptAt = others.indexOf(app.id);
          if (keptAt >= 0) {
            nodes.push(
              <DockSlot key={`gap-${app.id}`} open={gapAfter(keptAt)} jump={jump} testID={`dock-gap-${app.id}`} />,
            );
          }
          const icon = (
            <DockIcon
              id={app.id}
              name={app.name}
              testID={`dock-${app.id}`}
              label={app.name}
              status={running.has(app.id) ? runningWord : undefined}
              menu={menu?.id === app.id ? menu : null}
              setMenu={setMenu}
              activate={activateApp}
              runningColor={running.has(app.id) ? runningColor : undefined}
              rows={menu?.id === app.id ? rows : null}
              app={app}
              register={register}
              onFocusIcon={onFocusIcon}
              windowIds={windowIds.get(app.id)}
              MinimizeTarget={MinimizeTarget}
              arrangeActions={arrangeActions.get(app.id)}
              arrange={arrange}
              startDrag={startDrag}
            />
          );
          // Its scrolls, where the platform hands them; files dropped on it, where it opens them.
          const scrolled =
            scrollIcon && ScrollTarget ? (
              <IconScroll id={app.id} scroll={scrollIcon} ScrollTarget={ScrollTarget}>
                {icon}
              </IconScroll>
            ) : (
              icon
            );
          nodes.push(
            <DockSlot key={app.id} open={!(out && app.id === draggedKept)} jump={jump} testID={`dock-place-${app.id}`}>
              {openFiles ? (
                <AppDrop app={app} openFiles={openFiles}>
                  {scrolled}
                </AppDrop>
              ) : (
                scrolled
              )}
            </DockSlot>,
          );
          // The end of those kept: the gap there, before the applications only running.
          if (app.id === kept[kept.length - 1]) {
            nodes.push(<DockSlot key="gap-end" open={gapAfter(others.length)} jump={jump} testID="dock-gap-end" />);
          }
        }
        if (kept.length === 0) nodes.unshift(<DockSlot key="gap-end" open={gapAfter(0)} jump={jump} testID="dock-gap-end" />);
        return nodes;
      })()}
    </Box>
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
  announce,
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
  /** A press on an application's icon, and whether Option (Alt) was held at it: what a Mac's Dock
      does, and the frontmost application minimized into its icon (docs/adr/0034-the-dock.mdx,
      decision 9). The menu's Open launches with it too. */
  activateApp: (id: string, option: boolean) => void;
  /** The application's App Exposé (docs/adr/0045-app-expose.mdx): its windows alone, large. */
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
  /** Says a text to assistive technologies, politely (the platform's): an icon's new place once
      it moved from the keyboard or an assistive technology. */
  announce?: (text: string, polite: boolean) => void;
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
  // Arranging the dock by dragging (docs/adr/0034-the-dock.mdx, decision 10). `drag`: an
  // application dragged over the dock or out of it: its id when the shell knows it (its own drags,
  // from the dock or Launchpad; another application's only says its type before the drop), whether
  // it is the dock's own icon, and the gap it opens (its place among those kept), null while it is
  // off the dock. `pending`: the order the person dropped, shown until the session tells its own.
  // `jump`: rises when the dock takes a dropped order, everything then already where it goes.
  const platformDrag = usePlatformDrag();
  const [drag, setDrag] = useState<{ id: string | null; fromDock: boolean; gap: number | null } | null>(null);
  const dragRef = useRef(drag);
  dragRef.current = drag;
  const keptKey = keptIds({ applications, windows, activeAppId, openAtLogin: atLogin }).join("\n");
  const [pending, setPending] = useState<{ ids: string[]; base: string } | null>(null);
  const [jump, setJump] = useState(0);
  // The session told its list (whatever it kept): it is the dock's again.
  useEffect(() => {
    if (pending && pending.base !== keptKey) setPending(null);
  }, [keptKey, pending]);
  // One the session never answers (refused, or another shell's) does not stay.
  useEffect(() => {
    if (!pending) return undefined;
    const timer = setTimeout(() => setPending(null), 3000);
    return () => clearTimeout(timer);
  }, [pending]);
  // The applications as shown: those kept in the person's order (as dropped, until the session
  // tells it), then the others running.
  const shownApps = useMemo(() => {
    if (!pending) return apps;
    const kept = pending.ids.flatMap((id) => applications.filter((app) => app.id === id));
    return [...kept, ...apps.filter((app) => !pending.ids.includes(app.id) && running.has(app.id))];
  }, [apps, pending, applications, running]);
  const shownKept = useMemo(
    () => (pending ? pending.ids : shownApps.filter((app) => app.pinned).map((app) => app.id)),
    [pending, shownApps],
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
          return activateApp(id, false);
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
  // Arranging the dock without a pointer (docs/adr/0034-the-dock.mdx, decision 10): an icon kept
  // moves a place left or right, from the keyboard (Option and an arrow) or an assistive
  // technology's actions, which can also keep it or take it out of the dock. Its new place is said.
  const arrange = useCallback(
    (id: string, action: string) => {
      const state = latest.current;
      const app = state.applications.find((candidate) => candidate.id === id);
      if (!app || !actions.setDock) return false;
      if (action === "keep" || action === "remove") {
        actions.setDock(keepInDock(state, id, action === "keep"));
        return true;
      }
      const list = moveInDock(state, id, action === "moveLeft" ? -1 : 1);
      if (!list) return false;
      actions.setDock(list);
      const said = words.moved(app.name, list.indexOf(id) + 1, list.length);
      if (announce) announce(said, true);
      else AccessibilityInfo.announceForAccessibility(said);
      return true;
    },
    [actions, announce, words],
  );
  // Each icon's actions, the same array while its place among those kept is the same.
  const keptOrder = apps.filter((app) => app.pinned).map((app) => app.id).join("\n");
  const arrangeActions = useMemo(() => {
    const byId = new Map<string, AccessibilityActionInfo[]>();
    if (!actions.setDock) return byId;
    const kept = keptOrder ? keptOrder.split("\n") : [];
    for (const app of apps) {
      const at = kept.indexOf(app.id);
      byId.set(
        app.id,
        at < 0
          ? [{ name: "keep", label: words.keepInDock }]
          : [
              ...(at > 0 ? [{ name: "moveLeft", label: words.moveLeft }] : []),
              ...(at < kept.length - 1 ? [{ name: "moveRight", label: words.moveRight }] : []),
              { name: "remove", label: words.removeFromDock },
            ],
      );
    }
    return byId;
  }, [apps, keptOrder, actions, words]);
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
  const arrangeRef = useRef(arrange);
  arrangeRef.current = arrange;
  useEffect(() => {
    // An open menu's keys are its own.
    if (!keys || menu) return undefined;
    return keys.subscribe((event) => {
      // Option and an arrow move the focused icon a place along the dock, as kept.
      if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return false;
        const icon = focusedIcon.current;
        const id = icon?.startsWith("dock-") && icon !== "dock-trash" ? icon.slice(5) : null;
        if (id) arrangeRef.current(id, event.key === "ArrowLeft" ? "moveLeft" : "moveRight");
        // Taken either way: at an end it stays where it is, as nothing else moves.
        return true;
      }
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
  // The drag of an application over the dock. Where it would go is worked out from the dock as it
  // lays the places out with the gap open (and the dragged icon's own place closed): the gap is
  // then always under the pointer.
  const shelfWidth = useRef(0);
  const onShelfLayout = useCallback((event: { nativeEvent: { layout: { width: number } } }) => {
    shelfWidth.current = event.nativeEvent.layout.width;
  }, []);
  const latestShown = useRef({ shownApps, shownKept, trash: !!trash });
  latestShown.current = { shownApps, shownKept, trash: !!trash };
  const gapFor = useCallback((x: number, id: string | null) => {
    const { shownApps: shown, shownKept: kept, trash: hasTrash } = latestShown.current;
    const keptOthers = kept.filter((other) => other !== id).length;
    // The places open: every application's, less the dragged one's own if it is kept, and the gap.
    const places = shown.length - (id && kept.includes(id) ? 1 : 0) + 1;
    const row = 2 * ROW_PADDING + LAUNCHER_WIDTH + ICON_GAP + ICON + HALF_GAP + places * DOCK_SLOT +
      (hasTrash ? HALF_GAP + 1 + ICON_GAP + ICON : -HALF_GAP);
    const first = (shelfWidth.current - row) / 2 + ROW_PADDING + LAUNCHER_WIDTH + ICON_GAP + ICON + HALF_GAP;
    return gapAt(x - first, keptOthers);
  }, []);
  const accepts = actions.setDock !== undefined;
  const dragOver = useCallback(
    (event: DropTargetDragEvent) => {
      setDrag((now) => {
        const id = now?.id ?? shellDrag.current;
        const gap = gapFor(event.x, id);
        if (now && now.id === id && now.gap === gap) return now;
        return { id, fromDock: now?.fromDock ?? false, gap };
      });
    },
    [gapFor],
  );
  // Off the dock: the gap closes; the dock's own icon stays out of its place until the drag ends.
  const dragLeave = useCallback(() => {
    setDrag((now) => (now?.fromDock ? { ...now, gap: null } : null));
  }, []);
  // Dropped on the dock: kept where the gap is, in the person's list, which the session keeps; the
  // dock shows that order at once, the icon in the gap.
  const dropApp = useCallback(
    (event: DropTargetDropEvent) => {
      const id = event.data.other[APPLICATION_TYPE]?.trim();
      const now = dragRef.current;
      setDrag(null);
      const known = id && latest.current.applications.some((app) => app.id === id);
      if (!id || !known || !actions.setDock) return;
      const kept = latestShown.current.shownKept;
      const gap = now?.gap ?? gapFor(event.x, id);
      const ids = keptWithAt(kept, id, gap);
      setPending({ ids, base: keptKey });
      setJump((value) => value + 1);
      if (ids.join("\n") !== kept.join("\n")) actions.setDock(ids);
    },
    [actions, gapFor, keptKey],
  );
  // The dock's own icon dragged (a press moved past the slop): the platform drags its picture.
  // Its place closes and the gap opens where it was, at once, so nothing moves as it lifts off.
  // Released off the dock it is taken out: one not running vanishes in a poof with its sound, as
  // a Mac's, where it was let go; one running stays until it quits, after those kept, and its
  // picture goes back. Escape puts it back where it was.
  const startDrag = useCallback(
    (id: string, preview: React.RefObject<React.ComponentRef<typeof View> | null>) => {
      if (!platformDrag || !actions.setDock) return;
      const kept = latestShown.current.shownKept;
      const at = kept.indexOf(id);
      const isRunning = running.has(id);
      shellDrag.current = id;
      setDrag({ id, fromDock: true, gap: at >= 0 ? at : null });
      setJump((value) => value + 1);
      let reason: string | null = null;
      platformDrag
        .start({
          data: { other: { [APPLICATION_TYPE]: id } },
          actions: ["move"],
          previewRef: preview,
          vanish: !isRunning && at >= 0,
          onCancel: (why) => {
            reason = why;
          },
        })
        .then((action) => {
          if (action || reason !== "not-accepted" || at < 0) return;
          // Let go off the dock: out of it.
          const ids = latestShown.current.shownKept.filter((other) => other !== id);
          setPending({ ids, base: keptKey });
          actions.setDock?.(ids);
        })
        .catch((error: unknown) => console.warn("the dock could not drag", { id, error }))
        .finally(() => {
          if (shellDrag.current === id) shellDrag.current = null;
          setDrag((now) => (now?.fromDock && now.id === id ? null : now));
        });
    },
    [platformDrag, actions, running, keptKey],
  );
  const trashFiles = actions.trashFiles;
  const openFiles = actions.openFiles;
  const dividerStyle = useMemo(
    () => ({ width: 1, height: 28, marginRight: ICON_GAP, backgroundColor: colors.separator }),
    [colors],
  );
  // The shelf, made again only when what it shows changes: a window resized, moved or retitled,
  // or the pointer over an icon (whose own state it is), renders none of it.
  const shelf = useMemo(
    () => (
      <Scroll
        horizontal
        showsHorizontalScrollIndicator={false}
        // Without the Trash the last place's half space begins the row's end (24 in all).
        contentContainerClassName={`h-[55px] flex-row items-center justify-center ${trash ? "px-6" : "pl-6 pr-4"}`}
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
          style={OVERVIEW_PLACE}
          className="h-[42px] w-[42px] rounded-full border-0 p-0"
          iconSize={22}
          colors={{ background: colors.overview, foreground: colors.overviewIcon }}
        />
        <DockApps
          apps={shownApps}
          kept={shownKept}
          drag={drag}
          jump={jump}
          running={running}
          runningWord={words.running}
          runningColor={colors.running}
          menu={menu}
          rows={rows}
          setMenu={setMenu}
          activateApp={activateApp}
          register={register}
          onFocusIcon={onFocusIcon}
          windowIds={windowIds}
          MinimizeTarget={MinimizeTarget}
          arrangeActions={arrangeActions}
          arrange={arrange}
          startDrag={platformDrag && accepts ? startDrag : undefined}
          scrollIcon={scrollIcon}
          ScrollTarget={ScrollTarget}
          openFiles={openFiles}
        />
        {trash && (
          <Box style={TRASH_PLACE}>
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
          </Box>
        )}
      </Scroll>
    ),
    [shownApps, shownKept, drag, jump, running, colors, words, messages, view, menu, rows, trash, trashStatus, trashFiles, activateApp, openTrash, toggleLauncher, toggleOverview, dividerStyle, register, onFocusIcon, windowIds, MinimizeTarget, scrollIcon, ScrollTarget, arrangeActions, arrange, platformDrag, accepts, startDrag, openFiles],
  );
  return (
    <DockShelf color={colors.shelf} useDisplayCovered={useDisplayCovered}>
      {accepts ? (
        // The whole shelf takes an application dropped on it, to keep it where the gap is.
        <DropZone
          testID="dock-drop"
          accept={APP_ACCEPT}
          actions={APP_ACTIONS}
          onDragEnter={dragOver}
          onDragOver={dragOver}
          onDragLeave={dragLeave}
          onDrop={dropApp}
          style={SHELF_DROP}
        >
          <Box onLayout={onShelfLayout} style={SHELF_DROP}>
            {shelf}
          </Box>
        </DropZone>
      ) : (
        shelf
      )}
    </DockShelf>
  );
});
