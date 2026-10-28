import React, { useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { RiLayoutGridLine } from "@oxy.so/bloom/icons/RiLayoutGridLine";
import { RiCheckboxBlankCircleLine } from "@oxy.so/bloom/icons/RiCheckboxBlankCircleLine";
import { Box, Scroll } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellSnapshot } from "../model";

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
  const [hovered, setHovered] = useState<string | null>(null);
  const [rowWidth, setRowWidth] = useState(0);
  const [scrollOffset, setScrollOffset] = useState(0);
  const apps = snapshot.applications.filter(
    (app) => app.pinned || snapshot.windows.some((win) => win.appId === app.id),
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
      className="h-14 shrink-0 items-center justify-center border-t border-[#e5e9ef] bg-[#f7f9fc]"
      style={{ zIndex: 40 }}
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
          icon={RiCheckboxBlankCircleLine}
          accessibilityLabel="Abrir aplicaciones"
          appearance="plain"
          onPress={toggleLauncher}
          pressed={view === "launcher"}
          className="h-10 w-16 rounded-full border-0 bg-[#dfe6ed] p-0"
          iconSize={25}
          colors={{ background: "#dfe6ed", foreground: "#5b6871" }}
        />
        <Button
          testID="dock-overview"
          iconOnly
          icon={RiLayoutGridLine}
          accessibilityLabel="Ver escritorios"
          appearance="plain"
          onPress={toggleOverview}
          pressed={view === "overview"}
          className="h-[42px] w-[42px] rounded-full border-0 bg-[#e7edf3] p-0"
          iconSize={22}
          colors={{ background: "#e7edf3", foreground: "#56616a" }}
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
            {snapshot.windows.some((win) => win.appId === app.id) && (
              <Box className="absolute bottom-[2px] h-0.5 w-2.5 rounded-full bg-[#64717a]" />
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
            className="rounded-md border border-white/70 bg-[#f7f9fc] px-3 py-1.5"
            style={{ boxShadow: "0 3px 10px #17243b20" }}
          >
            <Text className="text-[12px] text-[#34404c]">
              {apps.find((app) => app.id === hovered)?.name}
            </Text>
          </Box>
        </Box>
      )}
    </Box>
  );
}
