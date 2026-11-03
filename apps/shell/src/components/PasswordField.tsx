import React, { useEffect, useRef, useState, type ComponentRef } from "react";
import { AccessibilityInfo, TextInput } from "react-native";
import Animated, {
  useAnimatedStyle,
  useSharedValue,
  withSequence,
  withTiming,
} from "react-native-reanimated";
import { Text } from "@oxy.so/bloom/typography";
import type { ShellLock } from "../model";
import { waitReminder, waitSaid } from "../lock-wait";
import { useShellMessages } from "../useShellMessages";

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
  placeholder,
  accessibilityLabel,
  announce,
  focusRequest = 0,
}: {
  /** Its field is `${testID}-password`, what it says `${testID}-hint`. */
  testID: string;
  /** Sends the password; false if it could not be sent. */
  authenticate: (password: string) => boolean;
  answer: ShellLock["answer"];
  capsLock?: boolean;
  /** Its placeholder and name; the catalogue's "Enter Password" and "Password" by default. */
  placeholder?: string;
  accessibilityLabel?: string;
  /** Says a text to assistive technologies (the platform's), else React Native's announcement. */
  announce?: (text: string, polite: boolean) => void;
  /** Changed, the field takes the focus again (the login screen's chosen account chosen again). */
  focusRequest?: number;
}) {
  const messages = useShellMessages();
  const words = messages.password;
  const [password, setPassword] = useState("");
  const [checking, setChecking] = useState(false);
  // What the last attempt said, by what it was, so that it is drawn in the language of the moment.
  const [message, setMessage] = useState<"incorrect" | "notChecked" | null>(null);
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
  // Shown, the field has the screen reader's focus as it has the keyboard's, as VoiceOver starts
  // on a Mac's password field: a reader on, or turned on here with Super+F5, says it.
  const input = useRef<ComponentRef<typeof TextInput>>(null);
  const focusedFor = useRef(focusRequest);
  useEffect(() => {
    if (!input.current) return;
    if (focusRequest !== focusedFor.current) {
      focusedFor.current = focusRequest;
      input.current.focus();
    }
    AccessibilityInfo.sendAccessibilityEvent(input.current, "focus");
  }, [focusRequest]);
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
    const waiting = answer.wait > 0 ? waitSaid(answer.wait, messages) : null;
    if (answer.outcome === "rejected") {
      setMessage("incorrect");
      say(waiting ? words.incorrectTryAgainIn(waiting) : words.incorrect, false);
      shake.value = withSequence(
        ...[-12, 12, -9, 9, -5, 5, 0].map((x) => withTiming(x, { duration: 45 })),
      );
    } else {
      if (waiting) say(words.tryAgainIn(waiting), false);
      // Throttled: the hint counts the wait down. Busy: another check was going on.
      // Accepted: the session takes over.
      setMessage(null);
    }
  }, [answer, shake]);
  useEffect(() => {
    if (counting.current > 0 && wait !== counting.current) {
      if (wait <= 0) {
        counting.current = 0;
        say(words.canTryAgain, true);
      } else if (waitReminder(wait)) {
        say(words.tryAgainIn(waitSaid(wait, messages)), true);
      }
    }
    if (wait <= 0) return undefined;
    const timer = setTimeout(() => setWait((left) => left - 1), 1000);
    return () => clearTimeout(timer);
  }, [wait]);
  // Caps Lock turned on is said, politely, where nothing else is under the field.
  const lastCapsLock = useRef(capsLock);
  useEffect(() => {
    if (capsLock && !lastCapsLock.current && wait <= 0 && !message) say(words.capsLock, true);
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
      setMessage("notChecked");
      say(words.notChecked, false);
    }
  };
  // What the last attempt said comes first; Caps Lock on is said while nothing else is.
  const hint =
    wait > 0
      ? words.tryAgainInSeconds(wait)
      : message
        ? words[message]
        : checking
          ? words.checking
          : capsLock
            ? words.capsLock
            : "";
  return (
    <>
      <Animated.View testID={`${testID}-field`} style={[{ marginTop: 12 }, field]}>
        <TextInput
          ref={input}
          testID={`${testID}-password`}
          accessibilityLabel={accessibilityLabel ?? words.label}
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
          placeholder={placeholder ?? words.placeholder}
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
          reads; each is announced once as it happens, above, never as a live region counting the
          wait down (an alert is one: it would say each second). */}
      <Text
        testID={`${testID}-hint`}
        style={{ ...textShadow, color: "rgba(255,255,255,0.85)", fontSize: 12, marginTop: 10, minHeight: 16 }}
      >
        {hint}
      </Text>
    </>
  );
}
