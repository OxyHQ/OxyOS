import React, { useState } from "react";
import { useColorScheme } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { RiLayoutGridLine } from "@oxy.so/bloom/icons/RiLayoutGridLine";
import { Box, Scroll } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellSnapshot } from "../model";
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

export function Dock({
  snapshot,
  view,
  activateApp,
  toggleLauncher,
  toggleOverview,
}: {
  snapshot: ShellSnapshot;
  view: string;
  activateApp: (id: string) => void;
  toggleLauncher: () => void;
  toggleOverview: () => void;
}) {
  const colors = DOCK_COLORS[useColorScheme() === "dark" ? "dark" : "light"];
  const [hovered, setHovered] = useState<string | null>(null);
  const [rowWidth, setRowWidth] = useState(0);
  const [scrollOffset, setScrollOffset] = useState(0);
  const apps = snapshot.applications.filter(
    (app) => app.pinned || snapshot.windows.some((win) => belongsTo(win, app.id)),
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
      style={{
        backgroundColor: colors.shelf,
        borderTopColor: colors.edge,
        height: DOCK_HEIGHT,
        borderTopLeftRadius: DOCK_HEIGHT / 2,
        borderTopRightRadius: DOCK_HEIGHT / 2,
        zIndex: 40,
      }}
      onLayout={(event) => setRowWidth(event.nativeEvent.layout.width)}
    >
      <Scroll
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="h-[55px] flex-row items-center justify-center gap-4 px-6"
        style={{ flexGrow: 0, maxWidth: "100%" }}
        onScroll={(event) => setScrollOffset(event.nativeEvent.contentOffset.x)}
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
          <Box
            key={app.id}
            className="h-[55px] w-[42px] items-center justify-center"
          >
            <Button
              testID={`dock-${app.id}`}
              accessibilityLabel={`Abrir ${app.name}`}
              appearance="plain"
              onPress={() => activateApp(app.id)}
              onHoverIn={() => setHovered(app.id)}
              onHoverOut={() => setHovered(null)}
              className="h-[42px] w-[42px] overflow-visible rounded-full border-0 bg-transparent p-0"
            >
              <AppIcon app={app} size={42} circular />
            </Button>
            {snapshot.windows.some((win) => belongsTo(win, app.id)) && (
              <Box
                className="absolute bottom-[2px] h-0.5 w-2.5 rounded-full"
                style={{ backgroundColor: colors.running }}
              />
            )}
          </Box>
        ))}
      </Scroll>
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
}
