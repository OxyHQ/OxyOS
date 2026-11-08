import React, { memo, useEffect, useRef, useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { Box } from "../primitives";
import type { ShellKeyboard } from "../platform";
import {
  bluetoothCodeTyped,
  sheetView,
  spokenCode,
  validBluetoothCode,
  validWifiPassword,
  type SheetText,
} from "../sheet";
import { useShellMessages } from "../useShellMessages";
import { useReducedTransparency } from "@oxy.so/bloom/theme";
import { glass } from "./glass";

export const SHEET_WIDTH = 380;
/** Every sheet is this tall, its words scrolling within it, and its buttons are this wide at its
    foot: Cancel and the answer are always in the same place, whatever the sheet says. */
export const SHEET_HEIGHT = 220;
export const SHEET_BUTTON_WIDTH = 120;
const INK = "#1d2430";
const QUIET = "#5b6472";
const ERROR = "#d93025";
/** The space after each digit of a code shown. */
const CODE_SPACING = 8;

/**
 * A system sheet (docs/adr/0030-installing-applications.mdx, decision 9): the session's question
 * to the person, over everything but the lock screen, in a shell surface no application draws,
 * covers or presses. Its answer is the shell's alone; Cancel is its first, safest button, and
 * Escape is Cancel, as on a Mac. No key accepts it by default (Return is no answer): only its
 * answer's button does, pressed, or reached with Tab and activated with Space as any control is
 * with Full Keyboard Access (docs/adr/0039-keyboard-access.mdx). It is said as it appears, as
 * VoiceOver says a Mac's alert: its title, what went wrong, and what it says. A sheet
 * that only tells (the install service is not running) has its one button, in the answer's place.
 *
 * A Wi-Fi password sheet (docs/adr/0038-networks.mdx) has a secure field, focused at once: its
 * answer is the password, by its Join button (enabled once the password can be one) or Return,
 * and goes to the session alone. The password lives in the field's own state and nowhere else.
 *
 * Pairing a Bluetooth device (docs/adr/0055-bluetooth.mdx), as a Mac's: a code to compare, or to
 * type on the device, is shown large, its digits spaced and said one by one; the code the device
 * shows, or its PIN, is typed in a field focused at once (a passkey's digits only), answered by
 * Pair (enabled once it can be one) or Return, and goes to the session alone. A code shown to type
 * on a device has Cancel alone, in the answer's place: the session takes the sheet away once the
 * device is paired.
 */
export const SystemSheet = memo(function SystemSheet({
  text,
  names,
  answer,
  answerPassword,
  answerCode,
  keyboard,
}: {
  text: SheetText;
  /** The registry's names, by id, for an application asking for a capability. */
  names?: Record<string, string>;
  answer: (accepted: boolean) => void;
  /** A password sheet's answer: the password typed. */
  answerPassword?: (password: string) => void;
  /** A Bluetooth pairing code's answer: the passkey or PIN typed. */
  answerCode?: (code: string) => void;
  /** The keys its surface receives, where the platform offers them. */
  keyboard?: ShellKeyboard;
}) {
  const reduced = useReducedTransparency();
  const messages = useShellMessages();
  const view = sheetView(text, messages, names);
  const [password, setPassword] = useState("");
  // Answered once: the session takes the sheet away, and a second press or Return sends nothing.
  const answered = useRef(false);
  const submit = (typed: string) => {
    if (answered.current || !answerPassword || !validWifiPassword(typed)) return;
    answered.current = true;
    setPassword("");
    answerPassword(typed);
  };
  const [code, setCode] = useState("");
  const codeKind = view.codeField?.kind;
  const submitCode = (typed: string) => {
    if (answered.current || !answerCode || !codeKind || !validBluetoothCode(typed, codeKind)) return;
    answered.current = true;
    setCode("");
    answerCode(typed);
  };
  const latestAnswer = useRef(answer);
  latestAnswer.current = answer;
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      const chord = event.altKey || event.ctrlKey || event.metaKey || event.shiftKey;
      if (event.key !== "Escape" || chord) return false;
      latestAnswer.current(false);
      return true;
    });
  }, [keyboard]);
  return (
    <Box
      testID="system-sheet-backdrop"
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: "#00000033",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Box
        testID="system-sheet"
        accessibilityViewIsModal
        accessibilityRole="alertdialog"
        accessibilityLabel={view.title}
        // Said as it appears, once, by the screen reader: its title, then what went wrong and what
        // it says, as VoiceOver reads an alert's message after its title.
        accessibilityHint={
          [view.error, view.body, view.code ? messages.sheet.bluetoothCodeSaid(spokenCode(view.code)) : ""]
            .filter(Boolean)
            .join(". ") || undefined
        }
        style={{
          width: SHEET_WIDTH,
          height: SHEET_HEIGHT,
          backgroundColor: glass("#ffffffee", reduced),
          borderRadius: 18,
          padding: 20,
          gap: 12,
        }}
      >
        <Box style={{ flex: 1, gap: 12, overflow: "hidden" }}>
        {/* The sheet is named by its title: the title is not read again as text. */}
        <Text importantForAccessibility="no-hide-descendants" style={{ fontSize: 17, fontWeight: "600", color: INK }}>
          {view.title}
        </Text>
        {view.error ? (
          <Text
            testID="system-sheet-error"
            // Said with the sheet as it appears, once: no live region (an alert would be one) says
            // it again. The sheet itself is the alert dialog.
            style={{ fontSize: 13, fontWeight: "600", color: ERROR }}
          >
            {view.error}
          </Text>
        ) : null}
        {view.body ? <Text style={{ fontSize: 13, color: QUIET }}>{view.body}</Text> : null}
        {view.password ? (
          <TextField>
            <TextFieldInput
              testID="system-sheet-password"
              label={view.password.label}
              placeholder={view.password.label}
              value={password}
              onChangeText={setPassword}
              secureTextEntry
              revealable
              revealLabels={{ show: view.password.show, hide: view.password.hide }}
              autoFocus
              autoCapitalize="none"
              autoCorrect={false}
              multiline={false}
              returnKeyType="join"
              submitBehavior="submit"
              // What the field holds as Return is pressed, as the lock screen sends its password.
              onSubmitEditing={({ nativeEvent }) => submit(nativeEvent.text)}
            />
          </TextField>
        ) : null}
        {view.code ? (
          <Text
            testID="system-sheet-code"
            // Said digit by digit, as VoiceOver says a pairing code.
            accessibilityLabel={spokenCode(view.code)}
            style={{
              fontSize: 34,
              fontWeight: "600",
              color: INK,
              textAlign: "center",
              letterSpacing: CODE_SPACING,
              // The spacing follows each digit, the last one too: as much before the first keeps it
              // centred.
              paddingLeft: CODE_SPACING,
              fontVariant: ["tabular-nums"],
            }}
          >
            {view.code}
          </Text>
        ) : null}
        {view.codeField && codeKind ? (
          <TextField>
            <TextFieldInput
              testID="system-sheet-code-field"
              label={view.codeField.label}
              placeholder={view.codeField.label}
              value={code}
              onChangeText={(typed) => setCode(bluetoothCodeTyped(typed, codeKind))}
              autoFocus
              autoCapitalize="none"
              autoCorrect={false}
              multiline={false}
              keyboardType={codeKind === "passkey" ? "number-pad" : "default"}
              inputMode={codeKind === "passkey" ? "numeric" : "text"}
              maxLength={codeKind === "passkey" ? 6 : 16}
              returnKeyType="done"
              submitBehavior="submit"
              // What the field holds as Return is pressed, as the password is sent.
              onSubmitEditing={({ nativeEvent }) => submitCode(bluetoothCodeTyped(nativeEvent.text, codeKind))}
            />
          </TextField>
        ) : null}
        {view.lines.map((line, index) => (
          <Box key={index} testID={`system-sheet-line-${index}`} style={{ gap: 2 }}>
            <Text style={{ fontSize: 14, color: INK }}>{line.label}</Text>
            {line.detail ? <Text style={{ fontSize: 12, color: QUIET }}>{line.detail}</Text> : null}
          </Box>
        ))}
        </Box>
        <Box style={{ flexDirection: "row", justifyContent: "flex-end", gap: 8 }}>
          {view.refuse ? (
            <Box style={{ width: SHEET_BUTTON_WIDTH }}>
              <Button testID="system-sheet-refuse" appearance="outline" tone="neutral" onPress={() => answer(false)}>
                {view.refuse}
              </Button>
            </Box>
          ) : null}
          {/* Without an answer (a code shown to type on a device), Cancel alone, where the answer is. */}
          {view.accept ? (
            <Box style={{ width: SHEET_BUTTON_WIDTH }}>
              <Button
                testID="system-sheet-accept"
                appearance="solid"
                tone={view.destructive ? "danger" : "accent"}
                disabled={
                  view.password
                    ? !answerPassword || !validWifiPassword(password)
                    : codeKind
                      ? !answerCode || !validBluetoothCode(code, codeKind)
                      : false
                }
                onPress={() => (view.password ? submit(password) : codeKind ? submitCode(code) : answer(true))}
              >
                {view.accept}
              </Button>
            </Box>
          ) : null}
        </Box>
      </Box>
    </Box>
  );
});
