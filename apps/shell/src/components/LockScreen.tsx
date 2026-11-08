import React, { memo, useRef, useState, type ComponentRef, type ComponentType } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { BlurTargetView } from "expo-blur";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { Wallpaper } from "./Wallpaper";
import type { ShellWallpaper } from "../wallpaper";
import { LockClock } from "./LockClock";
import { UserPicture } from "./UserPicture";
import { PasswordField, textShadow } from "./PasswordField";
import type { ShellApplication, ShellLock, ShellNotifications } from "../model";
import { lockSummary } from "../notifications";
import { AppIcon } from "../apps";
import type { MaskedProps } from "../platform";
import { useShellMessages } from "../useShellMessages";
import { SafeModeLabel } from "./SafeModeLabel";

/**
 * The backdrop of the lock and login screens: the wallpaper dimmed, and the date and the time
 * large in its material (LockClock), with `children` at the bottom.
 */
export function LockBackdrop({
  testID,
  wallpaper,
  now,
  Masked,
  clock = true,
  safeMode = false,
  children,
}: {
  testID: string;
  wallpaper: ShellWallpaper;
  now: Date;
  /** The platform's masked view, for the clock's material (LockClock). */
  Masked?: ComponentType<MaskedProps>;
  /** The date and the time over it; without them (the login screen's setup) the content takes
      the screen from near its top. */
  clock?: boolean;
  /** The session started in Safe Mode: said at the top right, as a Mac's login window says it
      (docs/adr/0054-recovery-and-erasing.mdx, decision 10). */
  safeMode?: boolean;
  children: React.ReactNode;
}) {
  const wallpaperTarget = useRef<ComponentRef<typeof View>>(null);
  return (
    <Box testID={testID} className="flex-1 overflow-hidden bg-black">
      {/* The wallpaper, which the clock's glyphs show blurred (expo-blur's target). */}
      <BlurTargetView ref={wallpaperTarget} style={StyleSheet.absoluteFill}>
        <Wallpaper wallpaper={wallpaper} decorative />
      </BlurTargetView>
      <Box
        className="absolute inset-0"
        style={{ backgroundColor: "rgba(0,0,0,0.28)" }}
      />
      {safeMode ? (
        <Box className="absolute" style={{ top: 10, right: 16 }}>
          <SafeModeLabel />
        </Box>
      ) : null}
      {clock ? (
        <>
          <Box className="items-center" style={{ marginTop: "9%" }}>
            <LockClock now={now} Masked={Masked} blurTarget={wallpaperTarget} />
          </Box>
          <Box className="absolute items-center" style={{ left: 0, right: 0, bottom: "11%" }}>
            {children}
          </Box>
        </>
      ) : (
        <Box className="absolute items-center" style={{ left: 0, right: 0, top: "8%" }}>
          {children}
        </Box>
      )}
    </Box>
  );
}

/**
 * How the person unlocks or signs in, as Windows Hello offers its PIN and the password: the PIN's
 * field while there is one (`usePassword` switches to the password's, when the password signs in
 * here), the password's otherwise; a locked-out PIN says to use the password, or, without one, to
 * sign in with Oxy at the login screen (no field: nothing here would open it). `testID`'s field is
 * `${testID}-pin` for the PIN.
 */
export function SecretFields({
  testID,
  pin,
  password = true,
  authenticate,
  authenticatePin,
  answer,
  capsLock,
  announce,
  focusRequest,
}: {
  testID: string;
  pin?: "set" | "locked";
  password?: boolean;
  authenticate: (password: string) => boolean;
  authenticatePin?: (pin: string) => boolean;
  answer: ShellLock["answer"];
  capsLock?: boolean;
  announce?: (text: string, polite: boolean) => void;
  focusRequest?: number;
}) {
  const words = useShellMessages().password;
  // The PIN first when there is one, as Windows Hello offers it.
  const [withPassword, setWithPassword] = useState(false);
  const usesPin = pin === "set" && !!authenticatePin && !(withPassword && password);
  const switchTo = (label: string, toPassword: boolean, id: string) => (
    <Pressable
      testID={id}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={() => setWithPassword(toPassword)}
      style={{ marginTop: 6, paddingHorizontal: 12, paddingVertical: 4 }}
    >
      <Text style={{ ...textShadow, color: "rgba(255,255,255,0.9)", fontSize: 13, fontWeight: "600" }}>
        {label}
      </Text>
    </Pressable>
  );
  if (pin === "locked" && !password) {
    return (
      <Text
        testID={`${testID}-pin-locked`}
        accessibilityRole="alert"
        style={{ ...textShadow, color: "#fff", fontSize: 13, marginTop: 12, maxWidth: 320, textAlign: "center" }}
      >
        {words.pinLockedNoPassword}
      </Text>
    );
  }
  if (usesPin) {
    return (
      <>
        <PasswordField
          key="pin"
          testID={`${testID}-pin`}
          passcode
          authenticate={authenticatePin}
          answer={answer}
          announce={announce}
          focusRequest={focusRequest}
        />
        {password ? switchTo(words.usePassword, true, `${testID}-use-password`) : null}
      </>
    );
  }
  return (
    <>
      <PasswordField
        key="password"
        testID={testID}
        authenticate={authenticate}
        answer={answer}
        capsLock={capsLock}
        announce={announce}
        focusRequest={focusRequest}
      />
      {pin === "locked" ? (
        <Text
          testID={`${testID}-pin-locked`}
          style={{ ...textShadow, color: "rgba(255,255,255,0.85)", fontSize: 12, maxWidth: 320, textAlign: "center" }}
        >
          {words.pinLocked}
        </Text>
      ) : pin === "set" && authenticatePin ? (
        switchTo(words.usePin, false, `${testID}-use-pin`)
      ) : null}
    </>
  );
}

/**
 * The lock screen (docs/adr/0010-lock-screen.mdx), as macOS's: the wallpaper dimmed, the date and
 * the time large, the user's picture and name, and the password field, or the PIN's (ADR 0020
 * decision 2). It decides nothing: what is typed goes to the session, and only the session
 * unlocks.
 */
export const LockScreen = memo(function LockScreen({
  lock,
  wallpaper,
  now,
  Masked,
  notifications,
  applications = [],
  announce,
  safeMode = false,
}: {
  lock: ShellLock;
  wallpaper: ShellWallpaper;
  now: Date;
  /** The platform's masked view, for the clock's material (LockClock). */
  Masked?: ComponentType<MaskedProps>;
  /** The session's notifications (docs/adr/0026-notifications.mdx): under the clock, how many each
      application has, and what its newest says only where the preferences allow previews. */
  notifications?: ShellNotifications;
  applications?: readonly ShellApplication[];
  /** Says a text to assistive technologies (the platform's), else React Native's announcement. */
  announce?: (text: string, polite: boolean) => void;
  /** The session started in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10). */
  safeMode?: boolean;
}) {
  const messages = useShellMessages();
  const name = lock.user.fullName || lock.user.name;
  const summary = notifications
    ? lockSummary(notifications.list, applications, notifications.previews, messages)
    : [];
  return (
    <LockBackdrop testID="shell-lock" wallpaper={wallpaper} now={now} Masked={Masked} safeMode={safeMode}>
      {summary.length > 0 ? (
        <Box
          testID="shell-lock-notifications"
          accessibilityRole="list"
          accessibilityLabel={messages.common.notifications}
          className="items-center gap-2"
          style={{ marginBottom: 28 }}
        >
          {summary.slice(0, 4).map((entry) => (
            <Box
              key={entry.appId}
              accessible
              accessibilityLabel={[entry.name, entry.count, entry.preview?.title, entry.preview?.body]
                .filter(Boolean)
                .join(". ")}
              className="flex-row items-center gap-3 rounded-[16px] px-3 py-2"
              style={{ width: 340, backgroundColor: "rgba(255,255,255,0.24)" }}
            >
              {entry.app ? <AppIcon app={entry.app} size={28} /> : null}
              <Box className="min-w-0 flex-1">
                <Text numberOfLines={1} style={{ ...textShadow, color: "#fff", fontSize: 12, fontWeight: "600" }}>
                  {entry.name}
                </Text>
                <Text numberOfLines={1} style={{ ...textShadow, color: "rgba(255,255,255,0.9)", fontSize: 12 }}>
                  {entry.preview ? `${entry.preview.title}: ${entry.preview.body}` : entry.count}
                </Text>
              </Box>
            </Box>
          ))}
        </Box>
      ) : null}
      <UserPicture />
      <Text
        style={{ ...textShadow, color: "#fff", fontSize: 15, fontWeight: "600", marginTop: 10 }}
      >
        {name}
      </Text>
      <SecretFields
        testID="shell-lock"
        pin={lock.pin}
        password={lock.password}
        authenticate={lock.authenticate}
        authenticatePin={lock.authenticatePin}
        answer={lock.answer}
        capsLock={lock.capsLock}
        announce={announce}
      />
    </LockBackdrop>
  );
});
