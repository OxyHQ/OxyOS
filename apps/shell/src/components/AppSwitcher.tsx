import React, { memo, useEffect } from "react";
import { AccessibilityInfo } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication } from "../model";
import type { Switcher } from "../switcher";

const ICON = 64;
const CELL = 88;

/**
 * The application switcher (docs/adr/0024-system-shortcuts.mdx), as a Mac's ⌘Tab: the running
 * applications' icons in a row, the most recently used first, the chosen one framed and named.
 * The keyboard drives it from the compositor (Super+Tab, Shift+Tab, `, the arrows, Q, H, Escape,
 * Super's release); a press on an icon activates that application at once. Each choice is said
 * to a screen reader, and every icon is a button naming its application, the chosen one selected.
 */
export const AppSwitcher = memo(function AppSwitcher({
  switcher,
  applications,
  activate,
}: {
  switcher: Switcher;
  applications: readonly ShellApplication[];
  activate: (id: string) => void;
}) {
  const apps = switcher.order
    .map((id) => applications.find((app) => app.id === id))
    .filter((app): app is ShellApplication => !!app);
  const chosen = apps[switcher.index];
  // Said once per choice, as VoiceOver says each application ⌘Tab moves to.
  const said = chosen?.name;
  useEffect(() => {
    if (said) AccessibilityInfo.announceForAccessibility(said);
  }, [said, switcher.index]);
  return (
    <Box
      testID="shell-switcher"
      accessibilityRole="list"
      accessibilityLabel="Cambiar de aplicación"
      className="flex-row items-start rounded-[28px] border border-white/60 p-3"
      style={{
        backgroundColor: "#eef1f6e6",
        boxShadow: "0 22px 70px #0b183a40, 0 0 0 .5px #0b183a1f",
        gap: 4,
      }}
    >
      {apps.map((app, index) => {
        const selected = index === switcher.index;
        return (
          <Box key={app.id} className="items-center" style={{ width: CELL }}>
            <Button
              testID={`switcher-${app.id}`}
              accessibilityLabel={app.name}
              appearance="plain"
              pressed={selected}
              onPress={() => activate(app.id)}
              className="items-center justify-center rounded-[20px] border-0 p-0"
              style={{ width: CELL, height: CELL }}
              colors={{
                background: selected ? "#0b183a1f" : "transparent",
                foreground: "#26313d",
              }}
            >
              <AppIcon app={app} size={ICON} />
            </Button>
            {/* The chosen one's name; the others keep its room, so the row never moves. */}
            <Box className="mt-1 h-[18px] items-center">
              {selected && (
                <Text
                  numberOfLines={1}
                  className="text-[12px] text-[#26313d]"
                  style={{ maxWidth: CELL }}
                >
                  {app.name}
                </Text>
              )}
            </Box>
          </Box>
        );
      })}
    </Box>
  );
});
