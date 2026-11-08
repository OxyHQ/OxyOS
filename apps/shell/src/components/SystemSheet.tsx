import React, { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Pressable } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { Box, Scroll, focusForAccessibility } from "../primitives";
import type { ShellKeyboard } from "../platform";
import {
  bluetoothCodeTyped,
  sheetView,
  spokenCode,
  submittablePassword,
  validBluetoothCode,
  type SheetAction,
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
/** A sheet that offers more than its answer (an application that quit: Reset…, Report…) is this
    wide, its other buttons at its foot's start; showing a report, it is this tall. */
export const WIDE_SHEET_WIDTH = 520;
export const REPORT_SHEET_HEIGHT = 420;
const INK = "#1d2430";
const BACKDROP = {
  position: "absolute",
  top: 0,
  left: 0,
  right: 0,
  bottom: 0,
  backgroundColor: "#00000033",
  alignItems: "center",
  justifyContent: "center",
} as const;
const CARD = { borderRadius: 18, padding: 20, gap: 12 } as const;
const QUIET = "#5b6472";
const ERROR = "#d93025";
/** Each of a passkey's accounts to choose from. */
const CHOICE_HEIGHT = 44;
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
 *
 * An application that quit unexpectedly (docs/adr/0061-crashes-and-resetting-an-application.mdx)
 * is told with Reopen, the answer the keyboard starts on (the platform's region puts its focus
 * there), Ignore, and at the foot's start Reset… (`act`, which the session confirms in a sheet of
 * its own) and Report…, which shows the report in the sheet itself: its text, which Copy puts on
 * the clipboard at the person's press, and Done, back to what it said (or away, when it opened at
 * the report); Escape is Done there.
 *
 * A passkey's sheet (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7) is answered by
 * the Oxy account's password, in the same secure field, focused at once, by Continue (enabled once
 * something is typed) or Return; it goes to the session alone, with the account chosen. When
 * several passkeys match, the accounts are a list of choices (radio buttons, the first chosen), ↑
 * and ↓ moving the choice, each said. A wrong password, or too many, asks again saying so.
 */
export const SystemSheet = memo(function SystemSheet({
  text,
  names,
  answer,
  answerPassword,
  answerCode,
  answerPasskey,
  act,
  copy,
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
  /** A passkey sheet's answer: the passkey chosen (its index, for a list of them) and the account's
      password typed. */
  answerPasskey?: (choice: number | undefined, password: string) => void;
  /** What else the sheet offers was pressed (Reset…): the sheet goes, as answered no. */
  act?: (action: SheetAction) => void;
  /** Puts a report's text on the clipboard, where the platform offers it. */
  copy?: (text: string) => Promise<boolean> | boolean | void;
  /** The keys its surface receives, where the platform offers them. */
  keyboard?: ShellKeyboard;
}) {
  const reduced = useReducedTransparency();
  const messages = useShellMessages();
  const view = sheetView(text, messages, names);
  const [password, setPassword] = useState("");
  // Answered once: the session takes the sheet away, and a second press or Return sends nothing.
  const answered = useRef(false);
  const passkey = text.kind === "passkey";
  const passwordAnswer = passkey ? answerPasskey : answerPassword;
  // Asked again (a wrong password), the account chosen before stays chosen.
  const [choice, setChoice] = useState(text.kind === "passkey" ? (text.chosen ?? 0) : 0);
  const choices = view.choices?.rows.length ?? 0;
  const submit = (typed: string) => {
    if (answered.current || !passwordAnswer || !submittablePassword(typed, view.passwordKind)) return;
    answered.current = true;
    setPassword("");
    if (passkey) answerPasskey?.(choices > 0 ? choice : undefined, typed);
    else answerPassword?.(typed);
  };
  const [code, setCode] = useState("");
  const codeKind = view.codeField?.kind;
  const submitCode = (typed: string) => {
    if (answered.current || !answerCode || !codeKind || !validBluetoothCode(typed, codeKind)) return;
    answered.current = true;
    setCode("");
    answerCode(typed);
  };
  // A report shown in the sheet itself, and whether its text was just copied.
  const [reading, setReading] = useState(view.report?.first ?? false);
  const [copied, setCopied] = useState(false);
  const done = useRef<React.ComponentRef<typeof Button>>(null);
  useLayoutEffect(() => {
    if (reading && !view.report?.first) focusForAccessibility(done.current as never);
  }, [reading, view.report?.first]);
  const closeReport = () => {
    if (view.report?.first) answer(false);
    else setReading(false);
  };
  const latestAnswer = useRef(answer);
  latestAnswer.current = answer;
  const latestEscape = useRef(() => latestAnswer.current(false));
  latestEscape.current = reading ? closeReport : () => latestAnswer.current(false);
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      const chord = event.altKey || event.ctrlKey || event.metaKey || event.shiftKey;
      if (chord) return false;
      // A passkey's accounts: ↑ and ↓ choose among them.
      if (choices > 1 && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        const step = event.key === "ArrowDown" ? 1 : -1;
        setChoice((current) => Math.min(choices - 1, Math.max(0, current + step)));
        return true;
      }
      if (event.key !== "Escape") return false;
      latestEscape.current();
      return true;
    });
  }, [keyboard, choices]);
  const wide = !!view.actions?.length;
  if (reading && view.report) {
    const report = view.report;
    return (
      <Box testID="system-sheet-backdrop" style={BACKDROP}>
        <Box
          testID="system-sheet"
          accessibilityViewIsModal
          accessibilityRole="alertdialog"
          accessibilityLabel={report.title}
          style={[
            CARD,
            { width: WIDE_SHEET_WIDTH, height: REPORT_SHEET_HEIGHT, backgroundColor: glass("#ffffffee", reduced) },
          ]}
        >
          <Text importantForAccessibility="no-hide-descendants" style={{ fontSize: 17, fontWeight: "600", color: INK }}>
            {report.title}
          </Text>
          <Scroll
            testID="system-sheet-report"
            style={{ flex: 1, backgroundColor: "#f3f4f6", borderRadius: 8 }}
            contentContainerStyle={{ padding: 10 }}
          >
            <Text selectable className="font-mono" style={{ fontSize: 11, lineHeight: 16, color: INK }}>
              {report.text}
            </Text>
          </Scroll>
          <Box style={{ flexDirection: "row", justifyContent: "flex-end", gap: 8 }}>
            <Box style={{ width: SHEET_BUTTON_WIDTH }}>
              <Button
                testID="system-sheet-copy"
                appearance="outline"
                tone="neutral"
                disabled={!copy}
                onPress={async () => setCopied((await copy?.(report.text)) !== false)}
              >
                {copied ? report.copied : report.copy}
              </Button>
            </Box>
            <Box style={{ width: SHEET_BUTTON_WIDTH }}>
              <Button ref={done} testID="system-sheet-done" appearance="solid" tone="accent" onPress={closeReport}>
                {report.done}
              </Button>
            </Box>
          </Box>
        </Box>
      </Box>
    );
  }
  return (
    <Box testID="system-sheet-backdrop" style={BACKDROP}>
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
        style={[
          CARD,
          {
            width: wide ? WIDE_SHEET_WIDTH : SHEET_WIDTH,
            // A passkey's accounts add a row each.
            height: SHEET_HEIGHT + choices * CHOICE_HEIGHT,
            backgroundColor: glass("#ffffffee", reduced),
          },
        ]}
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
              returnKeyType={passkey ? "go" : "join"}
              submitBehavior="submit"
              // What the field holds as Return is pressed, as the lock screen sends its password.
              onSubmitEditing={({ nativeEvent }) => submit(nativeEvent.text)}
            />
          </TextField>
        ) : null}
        {view.choices ? (
          <Box
            testID="system-sheet-choices"
            accessibilityRole="radiogroup"
            // Named by the account chosen, said politely as ↑ and ↓ change it.
            accessibilityLabel={`${view.choices.label}: ${view.choices.rows[choice]?.title ?? ""}`}
            accessibilityLiveRegion="polite"
            style={{ gap: 2 }}
          >
            {view.choices.rows.map((row, index) => (
              <Pressable
                key={index}
                testID={`system-sheet-choice-${index}`}
                accessibilityRole="radio"
                accessibilityLabel={`${row.title}, ${row.detail}`}
                accessibilityState={{ checked: index === choice, selected: index === choice }}
                onPress={() => setChoice(index)}
                style={{
                  height: CHOICE_HEIGHT - 2,
                  borderRadius: 8,
                  paddingHorizontal: 10,
                  justifyContent: "center",
                  backgroundColor: index === choice ? "#0a84ff22" : "transparent",
                }}
              >
                <Text importantForAccessibility="no-hide-descendants" style={{ fontSize: 14, color: INK }}>
                  {row.title}
                </Text>
                <Text importantForAccessibility="no-hide-descendants" style={{ fontSize: 12, color: QUIET }}>
                  {row.detail}
                </Text>
              </Pressable>
            ))}
          </Box>
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
          {/* What else it offers, at the foot's start: Reset… asks the session, Report… shows the
              report here. */}
          {view.actions?.length ? (
            <Box style={{ flexDirection: "row", gap: 4, marginRight: "auto" }}>
              {view.actions.map((action) => (
                <Button
                  key={action.id}
                  testID={`system-sheet-${action.id}`}
                  appearance="plain"
                  tone="neutral"
                  onPress={() => (action.id === "report" ? setReading(true) : act?.(action.id))}
                  disabled={action.id !== "report" && !act}
                >
                  {action.label}
                </Button>
              ))}
            </Box>
          ) : null}
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
                    ? !passwordAnswer || !submittablePassword(password, view.passwordKind)
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
