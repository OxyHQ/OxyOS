import React, { useEffect, useRef, useState } from "react";
import { TextInput } from "react-native";
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSequence,
  withTiming,
} from "react-native-reanimated";
import { Text } from "@oxy.so/bloom/typography";
import type { ShellLock } from "../model";

/** Linux-PAM's longest answer: the compositor takes no longer password (ADR 0010). */
const MAX_PASSWORD_BYTES = 512;
export const textShadow = {
  textShadowColor: "rgba(0,0,0,0.25)",
  textShadowOffset: { width: 0, height: 1 },
  textShadowRadius: 6,
};

/**
 * The password field of the lock and login screens, as macOS's: focused at once and through
 * every attempt, a secure field whose text goes to the session alone. A wrong password shakes it
 * and says so; while attempts must wait, it says how long; Caps Lock on is said under it. It
 * decides nothing: `authenticate` hands the password over, and `answer` is the session's.
 */
export function PasswordField({
  testID,
  authenticate,
  answer,
  capsLock,
  placeholder = "Introduce la contraseña",
  accessibilityLabel = "Contraseña",
}: {
  /** Its field is `${testID}-password`, what it says `${testID}-hint`. */
  testID: string;
  /** Sends the password; false if it could not be sent. */
  authenticate: (password: string) => boolean;
  answer: ShellLock["answer"];
  capsLock?: boolean;
  placeholder?: string;
  accessibilityLabel?: string;
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
  const lastAnswer = useRef(answer?.id);
  useEffect(() => {
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
      // Accepted: the session takes over.
      setMessage(null);
    }
  }, [answer, shake]);
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
    if (authenticate(typed)) setChecking(true);
    else setMessage("No se ha podido comprobar la contraseña");
  };
  // What the last attempt said comes first; Caps Lock on is said while nothing else is.
  const hint =
    wait > 0
      ? `Vuelve a intentarlo en ${wait} s`
      : (message ?? (checking ? "Comprobando…" : capsLock ? "Bloq Mayús activado" : ""));
  return (
    <>
      <Animated.View testID={`${testID}-field`} style={[{ marginTop: 12 }, field]}>
        <TextInput
          testID={`${testID}-password`}
          accessibilityLabel={accessibilityLabel}
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
          placeholder={placeholder}
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
        testID={`${testID}-hint`}
        accessibilityRole={hint ? "alert" : undefined}
        accessibilityLiveRegion="polite"
        style={{ ...textShadow, color: "rgba(255,255,255,0.85)", fontSize: 12, marginTop: 10, minHeight: 16 }}
      >
        {hint}
      </Text>
    </>
  );
}
