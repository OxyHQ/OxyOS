import * as React from "react";
import { View } from "react-native";
import Animated, {
  cancelAnimation,
  Easing,
  useAnimatedStyle,
  useSharedValue,
  withTiming,
  type SharedValue,
} from "react-native-reanimated";
import Svg, { Path, Rect } from "react-native-svg";
import { useTheme } from "@oxy.so/bloom/theme";
import {
  CYCLES,
  FINGER_RADIUS,
  fingersOf,
  frameAt,
  marksOf,
  PAD,
  PLAY_MS,
  type Finger,
  type Gesture,
} from "./gestures";

/** One finger, moved along its way as the play's frame says, on the UI thread. */
function FingerDot({
  finger,
  t,
  across,
  color,
}: {
  finger: Finger;
  t: SharedValue<number>;
  across: boolean;
  color: string;
}) {
  const moved = useAnimatedStyle(() => {
    const frame = frameAt(t.value);
    const along = frame.along * (across ? frame.direction : 1);
    return {
      opacity: frame.opacity,
      transform: [{ translateX: finger.dx * along }, { translateY: finger.dy * along }],
    };
  });
  return (
    <Animated.View
      style={[
        {
          position: "absolute",
          left: finger.x - FINGER_RADIUS,
          top: finger.y - FINGER_RADIUS,
          width: FINGER_RADIUS * 2,
          height: FINGER_RADIUS * 2,
          borderRadius: FINGER_RADIUS,
          backgroundColor: color,
        },
        moved,
      ]}
    />
  );
}

/**
 * A row's picture of its gesture in Trackpad's More Gestures (docs/adr/0046-interactive-gestures.mdx),
 * where a Mac's Trackpad settings plays a film of the hand: a small trackpad, the fingers on it and
 * the way they go, drawn still. It plays the gesture, twice over and then still again, when the
 * row's `choice` changes and each time `hovers` counts the pointer coming over the row (once a play
 * has ended: a pointer passing back and forth does not start it over). Off, it is dimmed and still;
 * with Reduce Motion (docs/adr/0019-settings.mdx) it never moves. Decorative: an assistive
 * technology reads the row by its title and its control alone.
 */
export function GestureIllustration({
  gesture,
  fingers,
  off,
  choice,
  hovers,
  reduceMotion,
}: {
  gesture: Gesture;
  fingers: 3 | 4;
  off: boolean;
  /** The row's choice: a change plays it. */
  choice: string;
  /** How many times the pointer has come over the row. */
  hovers: number;
  reduceMotion: boolean;
}) {
  const theme = useTheme();
  const t = useSharedValue(0);
  const playing = React.useRef(0);
  const still = off || reduceMotion;
  const play = React.useCallback(
    (over: boolean) => {
      if (still) return;
      const now = Date.now();
      if (!over && now < playing.current) return;
      playing.current = now + PLAY_MS;
      cancelAnimation(t);
      t.value = 0;
      t.value = withTiming(CYCLES, { duration: PLAY_MS, easing: Easing.linear });
    },
    [still, t],
  );
  // Turned off, or Reduce Motion turned on, mid-play: it rests at once.
  React.useEffect(() => {
    if (!still) return;
    cancelAnimation(t);
    t.value = 0;
    playing.current = 0;
  }, [still, t]);
  const shown = React.useRef(choice);
  React.useEffect(() => {
    if (shown.current === choice) return;
    shown.current = choice;
    play(true);
  }, [choice, play]);
  React.useEffect(() => {
    if (hovers > 0) play(false);
  }, [hovers, play]);

  const placed = fingersOf(gesture, fingers);
  return (
    <View
      accessible={false}
      aria-hidden
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      pointerEvents="none"
      style={{ width: PAD.width, height: PAD.height, opacity: off ? 0.35 : 1 }}
    >
      <Svg width={PAD.width} height={PAD.height} viewBox={`0 0 ${PAD.width} ${PAD.height}`}>
        <Rect
          x={0.5}
          y={0.5}
          width={PAD.width - 1}
          height={PAD.height - 1}
          rx={PAD.radius}
          fill={theme.colors.backgroundSecondary}
          stroke={theme.colors.border}
          strokeWidth={1}
        />
        {marksOf(gesture, placed).map((d) => (
          <Path
            key={d}
            d={d}
            fill="none"
            stroke={theme.colors.textSecondary}
            strokeOpacity={0.45}
            strokeWidth={1.5}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        ))}
      </Svg>
      {placed.map((finger, index) => (
        <FingerDot
          key={index}
          finger={finger}
          t={t}
          across={gesture === "desktops"}
          color={theme.colors.primary}
        />
      ))}
    </View>
  );
}
