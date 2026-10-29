import React, { useEffect, useRef, useState } from "react";
import { TextInput, type ImageSourcePropType } from "react-native";
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSequence,
  withTiming,
} from "react-native-reanimated";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { Wallpaper } from "./Wallpaper";
import type { ShellLock } from "../model";

/** Linux-PAM's longest answer: the compositor takes no longer password (ADR 0010). */
const MAX_PASSWORD_BYTES = 512;
const shadow = {
  textShadowColor: "rgba(0,0,0,0.25)",
  textShadowOffset: { width: 0, height: 1 },
  textShadowRadius: 6,
};

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]!.toUpperCase())
    .join("");

/**
 * The lock screen (docs/adr/0010-lock-screen.mdx), as macOS's: the wallpaper dimmed, the date and
 * the time large, the user's picture and name, and the password field. It decides nothing: the
 * password goes to the session, and only the session unlocks. A wrong one shakes the field and
 * says so; while attempts must wait, the field says how long.
 */
export function LockScreen({
  lock,
  wallpaper,
  now,
}: {
  lock: ShellLock;
  wallpaper: ImageSourcePropType;
  now: Date;
}) {
  const [password, setPassword] = useState("");
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  // Seconds until another attempt may be made, counted down.
  const [wait, setWait] = useState(0);
  const shake = useSharedValue(0);
  const field = useAnimatedStyle(() => ({
    transform: [{ translateX: shake.value }],
  }));
  const lastAnswer = useRef(lock.answer?.id);
  useEffect(() => {
    const answer = lock.answer;
    if (!answer || answer.id === lastAnswer.current) return;
    lastAnswer.current = answer.id;
    setChecking(false);
    setPassword("");
    setWait(answer.wait);
    if (answer.outcome === "rejected") {
      setMessage("Contraseña incorrecta");
      shake.value = withSequence(
        ...[-12, 12, -9, 9, -5, 5, 0].map((x) => withTiming(x, { duration: 45 })),
      );
    } else {
      // Throttled: the hint counts the wait down. Busy: another check was going on.
      setMessage(null);
    }
  }, [lock.answer, shake]);
  useEffect(() => {
    if (wait <= 0) return undefined;
    const timer = setTimeout(() => setWait((left) => left - 1), 1000);
    return () => clearTimeout(timer);
  }, [wait]);
  // The field stays focused and editable throughout, as macOS's: only submitting waits.
  // What the field holds as Enter is pressed (`nativeEvent.text`), not the state: typed quickly,
  // Enter arrives before the last characters' state is rendered.
  const submit = (typed: string) => {
    if (checking || wait > 0 || !typed) return;
    setMessage(null);
    if (lock.authenticate(typed)) setChecking(true);
    else setMessage("No se ha podido comprobar la contraseña");
  };
  const name = lock.user.fullName || lock.user.name;
  const date = now.toLocaleDateString("es", {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
  const time = now.toLocaleTimeString("es", {
    hour: "2-digit",
    minute: "2-digit",
  });
  // What the last attempt said comes first; Caps Lock on is said while nothing else is.
  const hint =
    wait > 0
      ? `Vuelve a intentarlo en ${wait} s`
      : (message ??
        (checking ? "Comprobando…" : lock.capsLock ? "Bloq Mayús activado" : ""));
  return (
    <Box testID="shell-lock" className="flex-1 overflow-hidden bg-black">
      <Box className="absolute inset-0">
        <Wallpaper source={wallpaper} decorative />
      </Box>
      <Box
        className="absolute inset-0"
        style={{ backgroundColor: "rgba(0,0,0,0.28)" }}
      />
      <Box className="items-center" style={{ marginTop: "9%" }}>
        <Text
          testID="shell-lock-date"
          style={{ ...shadow, color: "rgba(255,255,255,0.92)", fontSize: 22, fontWeight: "600" }}
        >
          {date.charAt(0).toUpperCase() + date.slice(1)}
        </Text>
        <Text
          testID="shell-lock-time"
          style={{
            ...shadow,
            color: "rgba(255,255,255,0.92)",
            fontSize: 112,
            lineHeight: 124,
            fontWeight: "700",
          }}
        >
          {time}
        </Text>
      </Box>
      <Box className="absolute items-center" style={{ left: 0, right: 0, bottom: "11%" }}>
        {/* The user's picture: their initials, which the name under it says in full. */}
        <Box
          className="items-center justify-center"
          importantForAccessibility="no-hide-descendants"
          accessibilityElementsHidden
          style={{
            width: 64,
            height: 64,
            borderRadius: 32,
            backgroundColor: "rgba(255,255,255,0.32)",
          }}
        >
          <Text style={{ color: "#fff", fontSize: 24, fontWeight: "600" }}>
            {initials(name)}
          </Text>
        </Box>
        <Text
          style={{ ...shadow, color: "#fff", fontSize: 15, fontWeight: "600", marginTop: 10 }}
        >
          {name}
        </Text>
        <Animated.View style={[{ marginTop: 12 }, field]}>
          <TextInput
            testID="shell-lock-password"
            accessibilityLabel="Contraseña"
            value={password}
            onChangeText={(text) => {
              setPassword(text);
              // Typing again clears what the last attempt said, as macOS does.
              setMessage(null);
            }}
            onSubmitEditing={({ nativeEvent }) => submit(nativeEvent.text)}
            secureTextEntry
            autoFocus
            maxLength={MAX_PASSWORD_BYTES}
            placeholder="Introduce la contraseña"
            placeholderTextColor="rgba(255,255,255,0.65)"
            returnKeyType="go"
            // Enter submits and the field stays focused for another try, as macOS's.
            submitBehavior="submit"
            style={{
              width: 200,
              height: 30,
              borderRadius: 15,
              paddingHorizontal: 14,
              backgroundColor: "rgba(255,255,255,0.22)",
              color: "#fff",
              fontSize: 13,
            }}
          />
        </Animated.View>
        {/* What the last attempt said, the wait, Caps Lock: an alert, which a screen reader
            announces as it appears. */}
        <Text
          testID="shell-lock-hint"
          accessibilityRole={hint ? "alert" : undefined}
          accessibilityLiveRegion="polite"
          style={{ ...shadow, color: "rgba(255,255,255,0.85)", fontSize: 12, marginTop: 10, minHeight: 16 }}
        >
          {hint}
        </Text>
      </Box>
    </Box>
  );
}
