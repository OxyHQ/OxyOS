/**
 * About This Computer's window (the Oxy menu's first row; a Mac's About This Mac, as macOS 27 draws
 * it): the system's sign, the computer's name, its processor, memory and OxyOS's version, and More
 * Info…, which opens Settings › About. A small window of a fixed size, moved by its background, with
 * the window's controls in their place; its facts are Settings › About's (`aboutThisComputer`, model.ts).
 */
import * as React from "react";
import { View } from "react-native";
import Svg, { Path } from "react-native-svg";
import { Button } from "@oxy.so/bloom/button";
import { SOCIAL_PROVIDERS } from "@oxy.so/bloom/social-button";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { useWindowSizeLimits, WindowControls, WindowDragRegion } from "@oxy.so/bloom/window";
import type { Messages } from "./messages";
import { aboutThisComputer, type SystemFacts } from "./model";

/** The window's size, as a Mac's About This Mac keeps one: it is neither resized nor zoomed. */
export const ABOUT_WINDOW = { width: 300, height: 380 } as const;
/** The sign's size, where a Mac draws the computer. */
const SIGN = 88;

const mark = SOCIAL_PROVIDERS.oxy;

export function AboutThisComputer({
  system,
  messages,
  moreInfo,
}: {
  system: SystemFacts;
  messages: Messages;
  /** Opens Settings › About. */
  moreInfo: () => void;
}) {
  const theme = useTheme();
  const about = aboutThisComputer(system, messages);
  useWindowSizeLimits({
    minWidth: ABOUT_WINDOW.width,
    maxWidth: ABOUT_WINDOW.width,
    minHeight: ABOUT_WINDOW.height,
    maxHeight: ABOUT_WINDOW.height,
  });
  return (
    <WindowDragRegion testID="about-this-computer" style={{ flex: 1, alignItems: "center", paddingTop: 44 }}>
      <View style={{ position: "absolute", top: 0, left: 0, right: 0 }} pointerEvents="box-none">
        <WindowControls />
      </View>
      <View accessible={false} importantForAccessibility="no-hide-descendants">
        <Svg width={SIGN} height={SIGN} viewBox={mark.viewBox}>
          <Path d={mark.path} fill={theme.colors.text} />
        </Svg>
      </View>
      {about.name !== "" && (
        <Text
          testID="about-this-computer-name"
          accessibilityRole="header"
          style={{ marginTop: 16, fontSize: 22, lineHeight: 26, fontWeight: "700", color: theme.colors.text }}
        >
          {about.name}
        </Text>
      )}
      <View style={{ marginTop: 18, width: "100%", paddingHorizontal: 20, gap: 4 }}>
        {about.rows.map((row) => (
          // One element per fact, its label and value read together.
          <View
            key={row.key}
            testID={`about-this-computer-${row.key}`}
            accessible
            accessibilityLabel={`${row.label}, ${row.value}`}
            style={{ flexDirection: "row", gap: 8 }}
          >
            <Text style={{ flex: 1, textAlign: "right", fontSize: 11, lineHeight: 14, color: theme.colors.text }}>
              {row.label}
            </Text>
            <Text
              numberOfLines={2}
              style={{ flex: 1, textAlign: "left", fontSize: 11, lineHeight: 14, color: theme.colors.textSecondary }}
            >
              {row.value}
            </Text>
          </View>
        ))}
      </View>
      <Button
        testID="about-this-computer-more-info"
        size="sm"
        appearance="outline"
        tone="neutral"
        onPress={moreInfo}
        style={{ marginTop: 22 }}
      >
        {about.moreInfo}
      </Button>
    </WindowDragRegion>
  );
}
