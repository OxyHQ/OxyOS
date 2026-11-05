import React, {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
} from "react";
import { Image, useWindowDimensions } from "react-native";
import { Text } from "@oxy.so/bloom/typography";
import { Box, PressArea } from "../primitives";
import { AppIcon } from "../apps";
import { Wallpaper } from "./Wallpaper";
import type { ImageSourcePropType } from "react-native";
import type { ShellActions, ShellApplication, ShellSnapshot, ShellWindow } from "../model";
import { exposedWindows } from "../expose";
import type { ShellKeyboard, WindowSize, WindowViewProps } from "../platform";
import { exposeLayout, neighbour, type Rect } from "../overview-geometry";
import { useShellMessages } from "../useShellMessages";

/** Where App Exposé lays its windows out, in its surface: the application's name above them, the
    minimized windows' row below, each window's name under it. */
const TITLE_TOP = 28;
const TITLE_HEIGHT = 44;
const SIDE = 48;
const GAP = 32;
const LABEL = 28;
const MINIMIZED_GAP = 24;
const MINIMIZED_LABEL = 22;
/** A window's corners in App Exposé, as a Mac's are drawn there. */
const RADIUS = 10;
/** The highlight round the window the pointer or the keyboard is on. */
const HIGHLIGHT = "#5b9bff";

const noSizes = (): ReadonlyMap<string, WindowSize> | undefined => undefined;

function aspect(size: WindowSize | undefined) {
  return size && size.width > 0 && size.height > 0
    ? { ratio: Math.max(0.3, Math.min(4, size.width / size.height)), height: size.height }
    : { ratio: 16 / 10 };
}

/** The area of App Exposé's windows a control is in: its open windows', or the minimized row. */
type Area = "open" | "minimized";

/**
 * One window in App Exposé: the window itself, live, where the layout puts it (the platform's
 * `WindowView`, into which the compositor moves the window from where it is,
 * docs/adr/0044-app-expose.mdx); one button named by its title and its place among the area's
 * windows ("2 de 3"), as VoiceOver reads App Exposé's windows. The pointer over it, or the
 * keyboard on it, rings it and shows its name under it, as a Mac's does. Memoized: the pointer
 * moving renders the window it enters and the one it leaves, nothing else.
 */
const ExposeWindow = memo(function ExposeWindow({
  window,
  app,
  rect,
  index,
  area,
  label,
  current,
  WindowView,
  choose,
  focused,
}: {
  window: ShellWindow;
  app?: ShellApplication;
  rect: Rect;
  index: number;
  area: Area;
  label: number;
  /** The area's window the keyboard reaches with Tab: only it is a stop, the arrows move it. */
  current: boolean;
  WindowView?: ComponentType<WindowViewProps>;
  choose: (window: ShellWindow) => void;
  focused: (area: Area, index: number) => void;
}) {
  const [hovered, setHovered] = useState(false);
  const [keyed, setKeyed] = useState(false);
  const lit = hovered || keyed;
  const onFocus = useCallback(() => {
    setKeyed(true);
    focused(area, index);
  }, [focused, area, index]);
  const onBlur = useCallback(() => setKeyed(false), []);
  const onPress = useCallback(() => choose(window), [choose, window]);
  const style = useMemo(
    () => ({
      position: "absolute" as const,
      left: rect.x,
      top: rect.y,
      width: rect.width,
      height: rect.height + label,
    }),
    [rect, label],
  );
  const view = useMemo(
    () => ({ width: rect.width, height: rect.height, borderRadius: RADIUS }),
    [rect.width, rect.height],
  );
  return (
    <Box style={style} pointerEvents="box-none">
      <PressArea
        testID={`expose-window-${window.id}`}
        accessibilityRole="button"
        accessibilityLabel={window.title}
        {...({
          accessibilityCollectionItem: {
            rowIndex: index,
            rowSpan: 1,
            columnIndex: 0,
            columnSpan: 1,
            heading: false,
          },
        } as object)}
        focusable={current}
        onFocus={onFocus}
        onBlur={onBlur}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        onPress={onPress}
        style={view}
      >
        {/* The window itself, live, where the platform shows it so; else a still picture of it, or
            its application's icon on a pane (the browser preview), as the overview's cards. */}
        {WindowView ? (
          <WindowView windowId={window.id} style={view} />
        ) : window.thumbnail ? (
          <Image source={window.thumbnail} resizeMode="contain" style={view} />
        ) : (
          <Box style={view} className="items-center justify-center bg-white/20">
            {app && <AppIcon app={app} size={48} />}
          </Box>
        )}
        {lit && (
          <Box
            pointerEvents="none"
            style={{
              position: "absolute",
              left: -4,
              top: -4,
              width: rect.width + 8,
              height: rect.height + 8,
              borderRadius: RADIUS + 4,
              borderWidth: 3,
              borderColor: HIGHLIGHT,
            }}
          />
        )}
      </PressArea>
      {/* Its title under it, which its button already says: no element of its own. */}
      {lit && (
        <Box importantForAccessibility="no-hide-descendants" accessibilityElementsHidden pointerEvents="none">
          <Text
            numberOfLines={1}
            className="mt-2 text-center text-[13px] font-medium text-white"
            style={{ width: rect.width, textShadowColor: "#00000080", textShadowRadius: 4 }}
          >
            {window.title}
          </Text>
        </Box>
      )}
    </Box>
  );
});

/**
 * The dimmed desktop under App Exposé (docs/adr/0044-app-expose.mdx), a backdrop surface of its
 * own, which the compositor fades in and out and never scales: the other applications' windows
 * fade away under it as the application's fly out to their places.
 */
export const AppExposeBackdrop = memo(function AppExposeBackdrop({
  wallpaper,
}: {
  wallpaper: ImageSourcePropType;
}) {
  return (
    <Box testID="app-expose-backdrop" pointerEvents="none" className="absolute inset-0 overflow-hidden">
      <Wallpaper source={wallpaper} decorative />
      <Box className="absolute inset-0 bg-[#0b0e14]/55" />
    </Box>
  );
});

/**
 * App Exposé, as a Mac's (docs/adr/0044-app-expose.mdx): one application's windows laid out large
 * across the screen, the others gone, its minimized windows in a row at the bottom, its name above.
 * Its windows are the windows themselves, moved into their places (the surface it is in moves
 * them, `movesWindows`): they fly there from where they are as it opens and back as it closes.
 * A window pressed comes forward and App Exposé closes; a minimized one is restored, out of the
 * dock; a press on nothing, Escape, or the gesture that opened it the other way closes it. The
 * arrows go from window to window, Tab between the open windows and the minimized row, Return
 * and Space choose. Laid out by the overview's rows (overview-geometry.ts).
 */
export const AppExpose = memo(function AppExpose({
  snapshot,
  appId,
  actions,
  dismiss,
  escapeRequest = 0,
  WindowView,
  useWindowSizes = noSizes,
  keyboard,
  focus,
  bottom = 0,
}: {
  snapshot: ShellSnapshot;
  appId: string;
  actions: ShellActions;
  dismiss: () => void;
  escapeRequest?: number;
  WindowView?: ComponentType<WindowViewProps>;
  useWindowSizes?: () => ReadonlyMap<string, WindowSize> | undefined;
  /** The keys before the focused control, while App Exposé has the keyboard: the arrows. */
  keyboard?: ShellKeyboard;
  /** Puts the keyboard's focus on the control of this testID. */
  focus?: (testID: string) => void;
  /** What of the screen its surface leaves below it (the dock). */
  bottom?: number;
}) {
  const messages = useShellMessages();
  const words = messages.appExpose;
  const sizes = useWindowSizes();
  const { width, height: screen } = useWindowDimensions();
  const height = screen - bottom;
  const app = snapshot.applications.find((candidate) => candidate.id === appId);
  const { open, minimized } = useMemo(() => exposedWindows(snapshot, appId), [snapshot, appId]);
  const sizeOf = (window: ShellWindow) => aspect(sizes?.get(window.id) ?? window.thumbnailSize);
  // The minimized windows' row: a sixth of the height, between 90 and 150 points.
  const rowHeight = minimized.length ? Math.round(Math.min(150, Math.max(90, height / 6))) : 0;
  const minimizedArea: Rect = {
    x: SIDE,
    y: height - rowHeight - 12,
    width: width - 2 * SIDE,
    height: rowHeight,
  };
  const openArea: Rect = {
    x: SIDE,
    y: TITLE_TOP + TITLE_HEIGHT + 16,
    width: width - 2 * SIDE,
    height: height - (TITLE_TOP + TITLE_HEIGHT + 16) - (rowHeight ? rowHeight + 2 * MINIMIZED_GAP : GAP),
  };
  const sizeKey = [...open, ...minimized]
    .map((window) => {
      const size = sizeOf(window);
      return `${window.id}:${size.ratio}:${size.height ?? 0}`;
    })
    .join(",");
  const openRects = useMemo(
    () => exposeLayout(open.map(sizeOf), openArea, GAP, LABEL),
    [sizeKey, width, height, rowHeight],
  );
  const minimizedRects = useMemo(
    () => exposeLayout(minimized.map(sizeOf), minimizedArea, MINIMIZED_GAP, MINIMIZED_LABEL),
    [sizeKey, width, height, rowHeight],
  );
  // The window each area's Tab stop is on, and where the keyboard is.
  const [current, setCurrent] = useState<Record<Area, number>>({ open: 0, minimized: 0 });
  const at = useRef<{ area: Area; index: number } | null>(null);
  const focused = useCallback((area: Area, index: number) => {
    at.current = { area, index };
    setCurrent((was) => (was[area] === index ? was : { ...was, [area]: index }));
  }, []);
  const latest = useRef({ open, minimized, openRects, minimizedRects, focus, snapshot });
  latest.current = { open, minimized, openRects, minimizedRects, focus, snapshot };
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      const direction = (
        { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" } as const
      )[event.key as "ArrowLeft"];
      if (!direction) return false;
      const now = latest.current;
      const place = at.current ?? (now.open.length ? { area: "open" as const, index: -1 } : { area: "minimized" as const, index: -1 });
      const windows = place.area === "open" ? now.open : now.minimized;
      const rects = place.area === "open" ? now.openRects : now.minimizedRects;
      if (!windows.length) return true;
      // Nothing yet: the arrows start on the area's first window.
      const next = place.index < 0 ? 0 : neighbour(rects, place.index, direction);
      const window = windows[next];
      if (window) {
        setCurrent((was) => ({ ...was, [place.area]: next }));
        now.focus?.(`expose-window-${window.id}`);
      }
      return true;
    });
  }, [keyboard]);
  const previousEscape = useRef(escapeRequest);
  useEffect(() => {
    if (previousEscape.current !== escapeRequest) dismiss();
    previousEscape.current = escapeRequest;
  }, [escapeRequest, dismiss]);
  const choose = useCallback(
    (window: ShellWindow) => {
      // A minimized window is restored, out of the dock; another desktop's is shown there.
      if (!window.minimized && window.workspaceId !== latest.current.snapshot.workspaceId) {
        actions.activateWorkspace(window.workspaceId);
      }
      actions.focusWindow(window.id);
      dismiss();
    },
    [actions, dismiss],
  );
  const collection = (count: number) =>
    ({ accessibilityCollection: { rowCount: count, columnCount: 1, hierarchical: false } }) as object;
  return (
    <Box
      testID="app-expose"
      // A dialog named by the application, said as it opens, then the window the keyboard is on.
      accessibilityRole="dialog"
      accessibilityLabel={app ? words.label(app.name) : words.title}
      className="absolute inset-0"
    >
      {/* Outside the windows: a press closes it, as Escape does. No stop of the keyboard's or the
          screen reader's (docs/adr/0039-keyboard-access.mdx). */}
      <PressArea accessible={false} onPress={dismiss} className="absolute inset-0" />
      {app && (
        <Box
          pointerEvents="none"
          className="absolute left-0 right-0 flex-row items-center justify-center gap-2.5"
          style={{ top: TITLE_TOP, height: TITLE_HEIGHT }}
        >
          <AppIcon app={app} size={30} />
          <Text
            accessibilityRole="header"
            className="text-[18px] font-semibold text-white"
            style={{ textShadowColor: "#00000080", textShadowRadius: 6 }}
          >
            {app.name}
          </Text>
        </Box>
      )}
      <Box
        testID="app-expose-windows"
        // The collection its windows say their place in ("2 de 3"), a list of its own.
        collapsable={false}
        accessibilityRole="list"
        accessibilityLabel={words.open}
        className="absolute inset-0"
        pointerEvents="box-none"
        {...collection(open.length)}
      >
        {open.map((window, index) =>
          openRects[index] ? (
            <ExposeWindow
              key={window.id}
              window={window}
              app={app}
              rect={openRects[index]!}
              index={index}
              area="open"
              label={LABEL}
              current={current.open === index}
              WindowView={WindowView}
              choose={choose}
              focused={focused}
            />
          ) : null,
        )}
      </Box>
      {minimized.length > 0 && (
        <>
          {/* The minimized windows apart, below a line, as a Mac's App Exposé keeps them. */}
          <Box
            pointerEvents="none"
            className="absolute bg-white/25"
            style={{ left: SIDE, right: SIDE, top: minimizedArea.y - MINIMIZED_GAP, height: 1 }}
          />
          <Box
            testID="app-expose-minimized"
            collapsable={false}
            accessibilityRole="list"
            accessibilityLabel={words.minimized}
            className="absolute inset-0"
            pointerEvents="box-none"
            {...collection(minimized.length)}
          >
            {minimized.map((window, index) =>
              minimizedRects[index] ? (
                <ExposeWindow
                  key={window.id}
                  window={window}
                  app={app}
                  rect={minimizedRects[index]!}
                  index={index}
                  area="minimized"
                  label={MINIMIZED_LABEL}
                  current={current.minimized === index}
                  WindowView={WindowView}
                  choose={choose}
                  focused={focused}
                />
              ) : null,
            )}
          </Box>
        </>
      )}
    </Box>
  );
});
