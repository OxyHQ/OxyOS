import React, { memo, useCallback, useMemo, useState } from "react";
import {
  useColorScheme,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { RiLayoutGridLine } from "@oxy.so/bloom/icons/RiLayoutGridLine";
import { Box, Scroll } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication, ShellWindow } from "../model";
import { OxyLogo } from "./OxyLogo";
import { belongsTo } from "../model";

/** The shelf's height: the work area ends above it, and the launcher's overlay with it. */
export const DOCK_HEIGHT = 56;

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
    tooltip: "#f7f9fc",
    tooltipEdge: "rgba(255,255,255,0.7)",
    tooltipText: "#34404c",
  },
  dark: {
    shelf: "#1d1f24",
    edge: "#2c2f36",
    launcher: "#2c3038",
    launcherIcon: "#c3cad3",
    overview: "#2a2e35",
    overviewIcon: "#c3cad3",
    running: "#aab3bd",
    tooltip: "#2a2d33",
    tooltipEdge: "rgba(255,255,255,0.12)",
    tooltipText: "#e6e9ee",
  },
} as const;

/** An application of the dock: its icon, and a dot under it while it has a window. Memoized: the
    pointer moving over the dock, or a window changing size, renders none of them again. */
const DockItem = memo(function DockItem({
  app,
  running,
  runningColor,
  activateApp,
  hover,
}: {
  app: ShellApplication;
  running: boolean;
  runningColor: string;
  activateApp: (id: string) => void;
  hover: (id: string | null) => void;
}) {
  const press = useCallback(() => activateApp(app.id), [activateApp, app.id]);
  const hoverIn = useCallback(() => hover(app.id), [hover, app.id]);
  const hoverOut = useCallback(() => hover(null), [hover]);
  return (
    <Box className="h-[55px] w-[42px] items-center justify-center">
      <Button
        testID={`dock-${app.id}`}
        accessibilityLabel={`Abrir ${app.name}`}
        appearance="plain"
        onPress={press}
        onHoverIn={hoverIn}
        onHoverOut={hoverOut}
        className="h-[42px] w-[42px] overflow-visible rounded-full border-0 bg-transparent p-0"
      >
        <AppIcon app={app} size={42} circular />
      </Button>
      {running && (
        <Box
          className="absolute bottom-[2px] h-0.5 w-2.5 rounded-full"
          style={{ backgroundColor: runningColor }}
        />
      )}
    </Box>
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

const SCROLL_STYLE = { flexGrow: 0, maxWidth: "100%" } as const;

export const Dock = memo(function Dock({
  applications,
  windows,
  view,
  activateApp,
  toggleLauncher,
  toggleOverview,
}: {
  applications: readonly ShellApplication[];
  windows: readonly ShellWindow[];
  view: string;
  activateApp: (id: string) => void;
  toggleLauncher: () => void;
  toggleOverview: () => void;
}) {
  const colors = DOCK_COLORS[useColorScheme() === "dark" ? "dark" : "light"];
  const [hovered, setHovered] = useState<string | null>(null);
  const [rowWidth, setRowWidth] = useState(0);
  const [scrollOffset, setScrollOffset] = useState(0);
  // What the shelf shows changes only with the applications that run: a window resized, moved or
  // retitled leaves it as it was.
  const runningIds = runningKey(applications, windows);
  const running = useMemo(() => new Set(runningIds ? runningIds.split("\n") : []), [runningIds]);
  const apps = useMemo(
    () => applications.filter((app) => app.pinned || running.has(app.id)),
    [applications, running],
  );
  const onLayout = useCallback(
    (event: LayoutChangeEvent) => setRowWidth(event.nativeEvent.layout.width),
    [],
  );
  const onScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => setScrollOffset(event.nativeEvent.contentOffset.x),
    [],
  );
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
  // The shelf, made again only when what it shows changes: the pointer over an icon changes the
  // tooltip alone.
  const shelf = useMemo(
    () => (
      <Scroll
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="h-[55px] flex-row items-center justify-center gap-4 px-6"
        style={SCROLL_STYLE}
        onScroll={onScroll}
        scrollEventThrottle={16}
      >
        <Button
          testID="dock-launcher"
          iconOnly
          icon={OxyLogo}
          accessibilityLabel="Abrir aplicaciones"
          appearance="plain"
          onPress={toggleLauncher}
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
          pressed={view === "overview"}
          className="h-[42px] w-[42px] rounded-full border-0 p-0"
          iconSize={22}
          colors={{ background: colors.overview, foreground: colors.overviewIcon }}
        />
        {apps.map((app) => (
          <DockItem
            key={app.id}
            app={app}
            running={running.has(app.id)}
            runningColor={colors.running}
            activateApp={activateApp}
            hover={setHovered}
          />
        ))}
      </Scroll>
    ),
    [apps, running, colors, view, activateApp, toggleLauncher, toggleOverview, onScroll],
  );
  // The tooltip shares the shelf coordinate space; scrolling never detaches it from its icon.
  const rowContentWidth =
    24 * 2 + 64 + 42 + apps.length * 42 + (apps.length + 1) * 16;
  const index = apps.findIndex((app) => app.id === hovered);
  const tooltipX = Math.max(
    64,
    Math.min(
      rowWidth - 64,
      (rowWidth - Math.min(rowWidth, rowContentWidth)) / 2 +
        24 +
        64 +
        16 +
        42 +
        16 +
        index * 58 +
        21 -
        scrollOffset,
    ),
  );
  return (
    <Box
      testID="shell-dock"
      className="shrink-0 items-center justify-center overflow-hidden border-t"
      // The shelf's top corners are fully rounded, as ChromeOS's shelf: the wallpaper shows
      // past them.
      style={shelfStyle}
      onLayout={onLayout}
    >
      {shelf}
      {hovered && (
        <Box
          pointerEvents="none"
          className="absolute -top-10 items-center"
          style={{ left: tooltipX - 64, width: 128 }}
        >
          <Box
            className="rounded-md border px-3 py-1.5"
            style={{
              backgroundColor: colors.tooltip,
              borderColor: colors.tooltipEdge,
              boxShadow: "0 3px 10px #17243b20",
            }}
          >
            <Text className="text-[12px]" style={{ color: colors.tooltipText }}>
              {apps.find((app) => app.id === hovered)?.name}
            </Text>
          </Box>
        </Box>
      )}
    </Box>
  );
});
