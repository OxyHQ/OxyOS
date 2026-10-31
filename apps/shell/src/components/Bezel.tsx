import React, { memo, useEffect, useRef, useState } from "react";
import { AccessibilityInfo, Animated } from "react-native";
import { RiVolumeUpLine } from "@oxy.so/bloom/icons/RiVolumeUpLine";
import { RiVolumeDownLine } from "@oxy.so/bloom/icons/RiVolumeDownLine";
import { RiVolumeMuteLine } from "@oxy.so/bloom/icons/RiVolumeMuteLine";
import { RiMicLine } from "@oxy.so/bloom/icons/RiMicLine";
import { RiMicOffLine } from "@oxy.so/bloom/icons/RiMicOffLine";
import { RiSunLine } from "@oxy.so/bloom/icons/RiSunLine";
import { RiKeyboardLine } from "@oxy.so/bloom/icons/RiKeyboardLine";
import { Box } from "../primitives";
import {
  BEZEL_MS,
  SEGMENTS,
  announcement,
  controlName,
  percent,
  segments,
  silent,
} from "../controls";
import type { ShellControl, ShellControls, ShellLevel } from "../model";

const SIZE = 200;
const SEGMENT = 9;
const GAP = 2;
/** How long it takes to fade once its time is up. */
const FADE_MS = 300;

function icon(control: ShellControl, level: ShellLevel) {
  switch (control) {
    case "output":
      if (silent(level)) return RiVolumeMuteLine;
      return level.level < 0.5 ? RiVolumeDownLine : RiVolumeUpLine;
    case "input":
      return level.muted ? RiMicOffLine : RiMicLine;
    case "display":
      return RiSunLine;
    default:
      return RiKeyboardLine;
  }
}

/**
 * A hardware key's bezel (docs/adr/0025-hardware-keys-and-control-center.mdx), as a Mac's: what
 * the key acted on and its level in sixteen segments, shown for a moment and faded, again with
 * every press. Its level is the session's as it is now, so it follows the system while shown.
 * It takes no input, and says the level to a screen reader once per press.
 */
export const Bezel = memo(function Bezel({
  bezel,
  levels,
}: {
  bezel?: ShellControls["bezel"];
  levels: ShellControls["levels"];
}) {
  const [shown, setShown] = useState<ShellControls["bezel"]>(undefined);
  const opacity = useRef(new Animated.Value(0)).current;
  const level = shown ? levels[shown.control] : undefined;
  // Each press shows it anew, for its time from that press.
  useEffect(() => {
    if (!bezel) return undefined;
    setShown(bezel);
    opacity.stopAnimation();
    opacity.setValue(1);
    const fade = setTimeout(() => {
      Animated.timing(opacity, {
        toValue: 0,
        duration: FADE_MS,
        useNativeDriver: true,
      }).start(({ finished }) => {
        if (finished) setShown(undefined);
      });
    }, BEZEL_MS);
    return () => clearTimeout(fade);
  }, [bezel?.id]);
  // Said once per press, after the session's answer is in the state.
  const said = shown && level ? announcement(shown.control, level) : undefined;
  useEffect(() => {
    if (said) AccessibilityInfo.announceForAccessibility(said);
  }, [shown?.id]);
  if (!shown || !level?.available) return null;
  const Icon = icon(shown.control, level);
  const filled = segments(level);
  return (
    <Box
      pointerEvents="none"
      style={{
        position: "absolute",
        left: 0,
        right: 0,
        bottom: 140,
        alignItems: "center",
      }}
    >
      <Animated.View
        testID="shell-bezel"
        accessible
        accessibilityRole="progressbar"
        accessibilityLabel={controlName(shown.control)}
        accessibilityValue={{
          min: 0,
          max: 100,
          now: level.muted ? 0 : percent(level.level),
          text: level.muted ? "Silenciado" : `${percent(level.level)} %`,
        }}
        style={{
          opacity,
          width: SIZE,
          height: SIZE,
          borderRadius: 36,
          backgroundColor: "#1b1f27b8",
          alignItems: "center",
          justifyContent: "center",
          gap: 26,
        }}
      >
        <Icon width={84} height={84} fill="#ffffff" />
        <Box style={{ flexDirection: "row", gap: GAP }}>
          {Array.from({ length: SEGMENTS }, (_, index) => (
            <Box
              key={index}
              testID={index < filled ? "bezel-filled" : "bezel-empty"}
              style={{
                width: SEGMENT,
                height: SEGMENT,
                borderRadius: 1.5,
                backgroundColor: index < filled ? "#ffffff" : "#ffffff33",
              }}
            />
          ))}
        </Box>
      </Animated.View>
    </Box>
  );
});
