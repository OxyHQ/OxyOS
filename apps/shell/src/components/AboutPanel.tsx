import React, { memo, useEffect, useRef } from "react";
import { useColorScheme } from "react-native";
import { CloseButton } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { useReducedTransparency } from "@oxy.so/bloom/theme";
import { Box } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication } from "../model";
import type { ShellKeyboard } from "../platform";
import { useShellMessages } from "../useShellMessages";
import { glass } from "./glass";

export const ABOUT_WIDTH = 280;
const ICON = 64;

/** What the standard About panel shows of an application: the registry's name, icon and
    version, the window's title for one that is no registry application's. */
export type AboutApplication = {
  name: string;
  application?: ShellApplication;
};

/**
 * The standard About panel (docs/adr/0021-menu-bar.mdx), a Mac's `orderFrontStandardAboutPanel`:
 * drawn by the system for an application that declares no About of its own, from what the
 * application registry says of it (its icon, its name, its version and build). It has the
 * keyboard while it shows; its close button, Escape or Ctrl+W (the applications' ⌘W) close it and
 * give the keyboard back to the application's window, and it closes when the person goes elsewhere.
 */
export const AboutPanel = memo(function AboutPanel({
  about,
  close,
  keyboard,
}: {
  about: AboutApplication;
  close: () => void;
  keyboard?: ShellKeyboard;
}) {
  const messages = useShellMessages();
  const words = messages.menus;
  const reduced = useReducedTransparency();
  const dark = useColorScheme() === "dark";
  const latestClose = useRef(close);
  latestClose.current = close;
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      const plain = !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
      const closeWindow = event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey && event.key.toLowerCase() === "w";
      if ((plain && event.key === "Escape") || closeWindow) {
        latestClose.current();
        return true;
      }
      return false;
    });
  }, [keyboard]);
  const { application, name } = about;
  const version = application?.version
    ? words.version(application.version, application.build)
    : undefined;
  const ink = dark ? "#f2f3f5" : "#1d2430";
  const quiet = dark ? "#a3a9b3" : "#5b6472";
  return (
    <Box
      testID="about-panel"
      accessibilityRole="none"
      accessibilityLabel={words.aboutApp(name)}
      style={{
        width: ABOUT_WIDTH,
        borderRadius: 16,
        paddingTop: 36,
        paddingBottom: 24,
        paddingHorizontal: 20,
        alignItems: "center",
        gap: 6,
        backgroundColor: glass(dark ? "#2a2c31ee" : "#ffffffee", reduced),
      }}
    >
      <CloseButton
        testID="about-panel-close"
        size="xs"
        accessibilityLabel={words.closeAbout}
        onPress={close}
        style={{ position: "absolute", top: 10, left: 10 }}
      />
      {application ? <AppIcon app={application} size={ICON} /> : <Box style={{ width: ICON, height: ICON }} />}
      <Text
        testID="about-panel-name"
        accessibilityRole="header"
        numberOfLines={2}
        style={{ marginTop: 8, fontSize: 15, lineHeight: 20, fontWeight: "700", color: ink, textAlign: "center" }}
      >
        {name}
      </Text>
      {version ? (
        <Text testID="about-panel-version" style={{ fontSize: 11, lineHeight: 14, color: quiet, textAlign: "center" }}>
          {version}
        </Text>
      ) : null}
    </Box>
  );
});
