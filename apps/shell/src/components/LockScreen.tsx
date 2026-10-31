import React, { memo, useRef, type ComponentRef, type ComponentType } from "react";
import { StyleSheet, View, type ImageSourcePropType } from "react-native";
import { BlurTargetView } from "expo-blur";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { Wallpaper } from "./Wallpaper";
import { LockClock } from "./LockClock";
import { UserPicture } from "./UserPicture";
import { PasswordField, textShadow } from "./PasswordField";
import type { ShellApplication, ShellLock, ShellNotifications } from "../model";
import { lockSummary } from "../notifications";
import { AppIcon } from "../apps";
import type { MaskedProps } from "../platform";

/**
 * The backdrop of the lock and login screens: the wallpaper dimmed, and the date and the time
 * large in its material (LockClock), with `children` at the bottom.
 */
export function LockBackdrop({
  testID,
  wallpaper,
  now,
  Masked,
  children,
}: {
  testID: string;
  wallpaper: ImageSourcePropType;
  now: Date;
  /** The platform's masked view, for the clock's material (LockClock). */
  Masked?: ComponentType<MaskedProps>;
  children: React.ReactNode;
}) {
  const wallpaperTarget = useRef<ComponentRef<typeof View>>(null);
  return (
    <Box testID={testID} className="flex-1 overflow-hidden bg-black">
      {/* The wallpaper, which the clock's glyphs show blurred (expo-blur's target). */}
      <BlurTargetView ref={wallpaperTarget} style={StyleSheet.absoluteFill}>
        <Wallpaper source={wallpaper} decorative />
      </BlurTargetView>
      <Box
        className="absolute inset-0"
        style={{ backgroundColor: "rgba(0,0,0,0.28)" }}
      />
      <Box className="items-center" style={{ marginTop: "9%" }}>
        <LockClock now={now} Masked={Masked} blurTarget={wallpaperTarget} />
      </Box>
      <Box className="absolute items-center" style={{ left: 0, right: 0, bottom: "11%" }}>
        {children}
      </Box>
    </Box>
  );
}

/**
 * The lock screen (docs/adr/0010-lock-screen.mdx), as macOS's: the wallpaper dimmed, the date and
 * the time large, the user's picture and name, and the password field. It decides nothing: the
 * password goes to the session, and only the session unlocks.
 */
export const LockScreen = memo(function LockScreen({
  lock,
  wallpaper,
  now,
  Masked,
  notifications,
  applications = [],
}: {
  lock: ShellLock;
  wallpaper: ImageSourcePropType;
  now: Date;
  /** The platform's masked view, for the clock's material (LockClock). */
  Masked?: ComponentType<MaskedProps>;
  /** The session's notifications (docs/adr/0026-notifications.mdx): under the clock, how many each
      application has, and what its newest says only where the preferences allow previews. */
  notifications?: ShellNotifications;
  applications?: readonly ShellApplication[];
}) {
  const name = lock.user.fullName || lock.user.name;
  const summary = notifications
    ? lockSummary(notifications.list, applications, notifications.previews)
    : [];
  return (
    <LockBackdrop testID="shell-lock" wallpaper={wallpaper} now={now} Masked={Masked}>
      {summary.length > 0 ? (
        <Box
          testID="shell-lock-notifications"
          accessibilityRole="list"
          accessibilityLabel="Notificaciones"
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
      <PasswordField
        testID="shell-lock"
        authenticate={lock.authenticate}
        answer={lock.answer}
        capsLock={lock.capsLock}
      />
    </LockBackdrop>
  );
});
