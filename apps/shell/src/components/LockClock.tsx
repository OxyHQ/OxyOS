import React, { useEffect, useState } from "react";
import type { ComponentRef, ComponentType, ReactNode, RefObject } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  useAnimatedStyle,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";
import { BlurView } from "expo-blur";
import { Text } from "@oxy.so/bloom/typography";
import type { MaskedProps } from "../platform";

const TIME_SIZE = 112;
const TIME_LINE = 124;
const DATE_SIZE = 22;
const DATE_LINE = 30;
/** Each digit in a box of its own, so that a changing one never moves the others. */
const DIGIT_WIDTH = 0.62;
const SEPARATOR_WIDTH = 0.3;
/** How far a digit travels as it changes, in its font size. */
const DIGIT_TRAVEL = 0.28;
const ease = Easing.out(Easing.cubic);

const glyphStyle = (size: number, line: number, width?: number) => ({
  color: "#fff",
  fontSize: size,
  lineHeight: line,
  fontWeight: "700" as const,
  textAlign: "center" as const,
  ...(width === undefined ? {} : { width }),
});

/**
 * One character of the time in its own box: when it changes, the old one rises and fades out as
 * the new one rises into its place, as a Mac's lock screen clock turns.
 */
function Glyph({ char }: { char: string }) {
  const [shown, setShown] = useState({
    current: char,
    previous: null as string | null,
    turn: 0,
  });
  useEffect(() => {
    setShown((last) =>
      last.current === char
        ? last
        : { current: char, previous: last.current, turn: last.turn + 1 },
    );
  }, [char]);
  const progress = useSharedValue(1);
  useEffect(() => {
    if (shown.turn === 0) return;
    progress.value = 0;
    progress.value = withTiming(1, { duration: 420, easing: ease });
  }, [shown.turn, progress]);
  const travel = TIME_SIZE * DIGIT_TRAVEL;
  const incoming = useAnimatedStyle(() => ({
    opacity: progress.value,
    transform: [{ translateY: (1 - progress.value) * travel }],
  }));
  const outgoing = useAnimatedStyle(() => ({
    opacity: 1 - progress.value,
    transform: [{ translateY: -progress.value * travel }],
  }));
  const width = TIME_SIZE * (/\d/.test(char) ? DIGIT_WIDTH : SEPARATOR_WIDTH);
  const style = glyphStyle(TIME_SIZE, TIME_LINE, width);
  return (
    <View style={{ width, height: TIME_LINE }}>
      {shown.previous !== null && (
        <Animated.View style={[StyleSheet.absoluteFill, outgoing]}>
          <Text style={style}>{shown.previous}</Text>
        </Animated.View>
      )}
      <Animated.View style={[StyleSheet.absoluteFill, incoming]}>
        <Text style={style}>{shown.current}</Text>
      </Animated.View>
    </View>
  );
}

/**
 * A line of the clock in the material of a Mac's lock screen: the wallpaper (`blurTarget`) seen
 * through its glyphs, blurred and lightened. Where the platform masks no views, the glyphs are
 * drawn white.
 */
function Material({
  Masked,
  blurTarget,
  height,
  children,
}: {
  Masked?: ComponentType<MaskedProps>;
  blurTarget: RefObject<ComponentRef<typeof View> | null>;
  height: number;
  children: ReactNode;
}) {
  const line = (
    <View
      style={{ height, flexDirection: "row", justifyContent: "center", alignItems: "center" }}
    >
      {children}
    </View>
  );
  if (!Masked) return line;
  return (
    <Masked mask={line} style={{ alignSelf: "stretch", height }}>
      <BlurView
        blurTarget={blurTarget}
        blurMethod="dimezisBlurView"
        intensity={85}
        blurReductionFactor={3.4}
        tint="light"
        style={StyleSheet.absoluteFill}
      />
    </Masked>
  );
}

/**
 * The lock screen's date and time (docs/adr/0010-lock-screen.mdx), as macOS Sonoma's: the time
 * large, each digit turning as it changes, both in the wallpaper's material; as the lock screen
 * appears they settle into place from a little above, fading in. A screen reader reads each line
 * whole.
 */
export function LockClock({
  now,
  Masked,
  blurTarget,
}: {
  now: Date;
  Masked?: ComponentType<MaskedProps>;
  blurTarget: RefObject<ComponentRef<typeof View> | null>;
}) {
  const date = now.toLocaleDateString("es", {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
  const day = date.charAt(0).toUpperCase() + date.slice(1);
  const time = now.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" });
  const shown = useSharedValue(0);
  useEffect(() => {
    shown.value = withTiming(1, { duration: 600, easing: ease });
  }, [shown]);
  const arriving = useAnimatedStyle(() => ({
    opacity: shown.value,
    transform: [{ translateY: (shown.value - 1) * 16 }],
  }));
  return (
    <Animated.View style={[{ alignSelf: "stretch" }, arriving]}>
      <View testID="shell-lock-date" accessible accessibilityRole="text" accessibilityLabel={day}>
        <Material Masked={Masked} blurTarget={blurTarget} height={DATE_LINE}>
          <Text style={glyphStyle(DATE_SIZE, DATE_LINE)}>{day}</Text>
        </Material>
      </View>
      <View testID="shell-lock-time" accessible accessibilityRole="text" accessibilityLabel={time}>
        <Material Masked={Masked} blurTarget={blurTarget} height={TIME_LINE}>
          {[...time].map((char, index) => (
            <Glyph key={index} char={char} />
          ))}
        </Material>
      </View>
    </Animated.View>
  );
}
