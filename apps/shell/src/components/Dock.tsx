import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Image, useColorScheme, type PointerEvent, type View } from "react-native";
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
import type { ShellKeyboard } from "../platform";
import { AppIcon } from "../apps";
import type { ShellActions, ShellApplication, ShellSnapshot, ShellWindow } from "../model";
import { OxyLogo } from "./OxyLogo";
import { TRASH_ICON } from "./trash-icon";
import { belongsTo } from "../model";
import {
  dockMenu,
  keepInDock,
  openAtLogin,
  trashMenu,
  trashedByDrop,
  type DockAbilities,
  type DockCommand,
  type DockMenuItem,
} from "../dock";

/** The shelf's height: the work area ends above it, and the launcher's overlay with it. */
export const DOCK_HEIGHT = 56;

/** An icon's side, and the shelf's row it sits in. */
const ICON = 42;
const ROW = 55;
/** How far above its icon a menu opens, as a Mac's Dock leaves a gap. */
const MENU_GAP = 10;
/** The Trash's place among the dock's menus. */
const TRASH = "trash";

/** The shelf's colours in the system's appearance (docs/adr/0019-settings.mdx), as a Mac's dock
    turns dark with it. */
const DOCK_COLORS = {
  light: {
    shelf: "#f7f9fc",
    edge: "#e5e9ef",
    launcher: "#dfe6ed",
    launcherIcon: "#5b6871",
    overview: "#e7edf3",
    overviewIcon: "#56616a",
    running: "#64717a",
    separator: "#d5dbe2",
  },
  dark: {
    shelf: "#1d1f24",
    edge: "#2c2f36",
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
const TRASH_IMAGE = { width: ICON, height: ICON } as const;
const TRASH_ROUND = { width: ICON, height: ICON, borderRadius: ICON / 2 } as const;
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
 * Memoized with handlers that stay the same: the pointer moving over the dock, or a window changing
 * size, renders none of them again, and hovering one renders it alone.
 */
const DockIcon = memo(function DockIcon({
  id,
  name,
  testID,
  label,
  menu,
  setMenu,
  activate,
  runningColor,
  rows,
  app,
  register,
  onFocusIcon,
}: {
  id: string;
  name: string;
  testID: string;
  label: string;
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
}) {
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
  // The icon's button, made again only when what it shows or does changes: the tooltip showing or
  // going renders the tooltip, not the button (whose own hover is Bloom's).
  const trigger = useMemo(
    () => (
      <ContextMenuTrigger asChild>
        <Button
          testID={testID}
          accessibilityLabel={label}
          appearance="plain"
          onPress={press}
          onPressIn={pressIn}
          onHoverIn={hoverIn}
          onHoverOut={hoverOut}
          onFocus={onFocus}
          className="h-[42px] w-[42px] overflow-visible rounded-full border-0 bg-transparent p-0"
        >
          {app ? (
            <AppIcon app={app} size={ICON} circular />
          ) : (
            <Box className="items-center justify-center overflow-hidden" style={TRASH_ROUND}>
              <Image source={TRASH_ICON} resizeMode="cover" style={TRASH_IMAGE} />
            </Box>
          )}
        </Button>
      </ContextMenuTrigger>
    ),
    [testID, label, press, pressIn, hoverIn, hoverOut, onFocus, app],
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

const TRASH_ACCEPT = ["files"] as const;
const TRASH_ACTIONS = ["move"] as const;
const TRASH_TARGETED = { backgroundColor: "rgba(0, 0, 0, 0.22)" } as const;

/**
 * The Trash as a drop target (docs/adr/0035-drag-and-drop.mdx): files dragged from any
 * application onto it, with a move alone (a Mac's Trash), are put in it by the platform; while a
 * drag it would take is over it, its row darkens, as a Mac's Dock marks the Trash it is over.
 */
const TrashDrop = memo(function TrashDrop({
  trashFiles,
  children,
}: {
  trashFiles: (uris: readonly string[]) => void;
  children: React.ReactNode;
}) {
  const [targeted, setTargeted] = useState(false);
  const enter = useCallback(() => setTargeted(true), []);
  const leave = useCallback(() => setTargeted(false), []);
  const drop = useCallback(
    ({ data }: DropTargetDropEvent) => {
      setTargeted(false);
      const uris = trashedByDrop(data.files);
      if (uris.length > 0) trashFiles(uris);
    },
    [trashFiles],
  );
  return (
    <DropZone testID="dock-trash-drop" accept={TRASH_ACCEPT} actions={TRASH_ACTIONS} onDragEnter={enter} onDragLeave={leave} onDrop={drop}>
      <Box testID={targeted ? "dock-trash-targeted" : undefined} className="rounded-full" style={targeted ? TRASH_TARGETED : undefined}>
        {children}
      </Box>
    </DropZone>
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
  /** The application's windows alone in the overview, as a Mac's App Exposé. */
  showAllWindows?: (id: string) => void;
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
}) {
  const colors = DOCK_COLORS[useColorScheme() === "dark" ? "dark" : "light"];
  const [menu, setMenu] = useState<OpenMenu | null>(null);
  // What the shelf shows changes only with the applications that run: a window resized, moved or
  // retitled leaves it as it was.
  const runningIds = runningKey(applications, windows);
  const running = useMemo(() => new Set(runningIds ? runningIds.split("\n") : []), [runningIds]);
  const apps = useMemo(
    () => applications.filter((app) => app.pinned || running.has(app.id)),
    [applications, running],
  );
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
      return trashMenu(trash?.items, !!actions.emptyTrash).map((entry) => (
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
      menu.option,
    );
    return <MenuRows items={items} run={(command) => run(menu.id, command)} />;
  }, [menu, applications, windows, activeAppId, atLogin, abilities, trash, actions, run]);
  const openTrash = useCallback(() => actions.openTrash?.(), [actions]);
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
  const shelfStyle = useMemo(
    () => ({
      backgroundColor: colors.shelf,
      borderTopColor: colors.edge,
      height: DOCK_HEIGHT,
      borderTopLeftRadius: DOCK_HEIGHT / 2,
      borderTopRightRadius: DOCK_HEIGHT / 2,
      zIndex: 40,
    }),
    [colors],
  );
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
          accessibilityLabel="Abrir aplicaciones"
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
          accessibilityLabel="Ver escritorios"
          appearance="plain"
          onPress={toggleOverview}
          onFocus={() => onFocusIcon("dock-overview")}
          pressed={view === "overview"}
          className="h-[42px] w-[42px] rounded-full border-0 p-0"
          iconSize={22}
          colors={{ background: colors.overview, foreground: colors.overviewIcon }}
        />
        {apps.map((app) => (
          <DockIcon
            key={app.id}
            id={app.id}
            name={app.name}
            testID={`dock-${app.id}`}
            label={`Abrir ${app.name}`}
            menu={menu?.id === app.id ? menu : null}
            setMenu={setMenu}
            activate={activateApp}
            runningColor={running.has(app.id) ? colors.running : undefined}
            rows={menu?.id === app.id ? rows : null}
            app={app}
            register={register}
            onFocusIcon={onFocusIcon}
          />
        ))}
        {trash && (
          <>
            {/* The Trash sits past a divider at the dock's end, as a Mac's. */}
            <Box style={dividerStyle} />
            {trashFiles ? (
              <TrashDrop trashFiles={trashFiles}>
                <DockIcon
                  id={TRASH}
                  name="Papelera"
                  testID="dock-trash"
                  label="Abrir Papelera"
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
                name="Papelera"
                testID="dock-trash"
                label="Abrir Papelera"
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
    [apps, running, colors, view, menu, rows, trash, trashFiles, activateApp, openTrash, toggleLauncher, toggleOverview, dividerStyle, register, onFocusIcon],
  );
  return (
    <Box
      testID="shell-dock"
      className="shrink-0 items-center justify-center overflow-hidden border-t"
      // The shelf's top corners are fully rounded, as ChromeOS's shelf: the wallpaper shows
      // past them.
      style={shelfStyle}
    >
      {shelf}
    </Box>
  );
});
