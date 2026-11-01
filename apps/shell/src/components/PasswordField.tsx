import React, { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, TextInput } from "react-native";
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

/** A wait as said: "30 segundos", "2 minutos" (the visible hint counts it in seconds). */
export function waitSaid(seconds: number): string {
  if (seconds < 60 || seconds % 60 !== 0) return seconds === 1 ? "1 segundo" : `${seconds} segundos`;
  const minutes = seconds / 60;
  return minutes === 1 ? "1 minuto" : `${minutes} minutos`;
}

/**
 * Whether a wait counting down is said again with `left` seconds left: each whole minute, then
 * half a minute and ten seconds before the end. Never each second: VoiceOver says a status text
 * when it appears and not again as it changes in place (a Mac's lock says its wait once), and a
 * count said every second would drown out the typing echo and the person's own reading. These
 * few reminders and the moment the wait ends tell someone who cannot see the count when to try.
 */
export function waitReminder(left: number): boolean {
  return left === 30 || left === 10 || (left > 0 && left % 60 === 0);
}

/**
 * The password field of the lock and login screens, as macOS's: focused at once and through
 * every attempt, a secure field whose text goes to the session alone. A wrong password shakes it
 * and says so; while attempts must wait, it says how long; Caps Lock on is said under it. It
 * decides nothing: `authenticate` hands the password over, and `answer` is the session's.
 *
 * What it says under the field is told to assistive technologies once, as it happens, by
 * announcements rather than a live region (which would say the countdown every second): an
 * attempt's answer at once; Caps Lock turned on, a wait's reminders (`waitReminder`) and its end
 * politely.
 */
export function PasswordField({
  testID,
  authenticate,
  answer,
  capsLock,
  placeholder = "Introduce la contraseña",
  accessibilityLabel = "Contraseña",
  announce,
}: {
  /** Its field is `${testID}-password`, what it says `${testID}-hint`. */
  testID: string;
  /** Sends the password; false if it could not be sent. */
  authenticate: (password: string) => boolean;
  answer: ShellLock["answer"];
  capsLock?: boolean;
  placeholder?: string;
  accessibilityLabel?: string;
  /** Says a text to assistive technologies (the platform's), else React Native's announcement. */
  announce?: (text: string, polite: boolean) => void;
}) {
  const [password, setPassword] = useState("");
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  // Seconds until another attempt may be made, counted down.
  const [wait, setWait] = useState(0);
  const latestAnnounce = useRef(announce);
  latestAnnounce.current = announce;
  const say = (text: string, polite: boolean) => {
    if (latestAnnounce.current) latestAnnounce.current(text, polite);
    else AccessibilityInfo.announceForAccessibility(text);
  };
  // The wait being counted down, from the session's answer: its start is said with the answer,
  // its reminders and its end as it counts.
  const counting = useRef(0);
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
    counting.current = answer.wait;
    const waiting = answer.wait > 0 ? `Vuelve a intentarlo en ${waitSaid(answer.wait)}` : null;
    if (answer.outcome === "rejected") {
      setMessage("Contraseña incorrecta");
      say(waiting ? `Contraseña incorrecta. ${waiting}` : "Contraseña incorrecta", false);
      shake.value = withSequence(
        ...[-12, 12, -9, 9, -5, 5, 0].map((x) => withTiming(x, { duration: 45 })),
      );
    } else {
      if (waiting) say(waiting, false);
      // Throttled: the hint counts the wait down. Busy: another check was going on.
      // Accepted: the session takes over.
      setMessage(null);
    }
  }, [answer, shake]);
  useEffect(() => {
    if (counting.current > 0 && wait !== counting.current) {
      if (wait <= 0) {
        counting.current = 0;
        say("Ya puedes volver a intentarlo", true);
      } else if (waitReminder(wait)) {
        say(`Vuelve a intentarlo en ${waitSaid(wait)}`, true);
      }
    }
    if (wait <= 0) return undefined;
    const timer = setTimeout(() => setWait((left) => left - 1), 1000);
    return () => clearTimeout(timer);
  }, [wait]);
  // Caps Lock turned on is said, politely, where nothing else is under the field.
  const lastCapsLock = useRef(capsLock);
  useEffect(() => {
    if (capsLock && !lastCapsLock.current && wait <= 0 && !message) say("Bloq Mayús activado", true);
    lastCapsLock.current = capsLock;
  }, [capsLock]);
  // The field stays focused and editable throughout, as macOS's: only submitting waits.
  // What the field holds as Enter is pressed (`nativeEvent.text`), not the state: typed quickly,
  // Enter arrives before the last characters' state is rendered.
  const submit = (typed: string) => {
    if (checking || wait > 0 || !typed) return;
    setMessage(null);
    if (authenticate(typed)) setChecking(true);
    else {
      setMessage("No se ha podido comprobar la contraseña");
      say("No se ha podido comprobar la contraseña", false);
    }
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
      {/* What the last attempt said, the wait, Caps Lock, as text the screen reader's cursor
          reads (an attempt's answer as an alert); each is announced once as it happens, above,
          never as a live region counting the wait down. */}
      <Text
        testID={`${testID}-hint`}
        accessibilityRole={wait > 0 || message ? "alert" : undefined}
        style={{ ...textShadow, color: "rgba(255,255,255,0.85)", fontSize: 12, marginTop: 10, minHeight: 16 }}
      >
        {hint}
      </Text>
    </>
  );
}
