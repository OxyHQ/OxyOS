import React, { useRef, type ComponentRef, type ComponentType } from "react";
import { StyleSheet, View, type ImageSourcePropType } from "react-native";
import { BlurTargetView } from "expo-blur";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { Wallpaper } from "./Wallpaper";
import { LockClock } from "./LockClock";
import { UserPicture } from "./UserPicture";
import { PasswordField, textShadow } from "./PasswordField";
import type { ShellLock } from "../model";
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
export function LockScreen({
  lock,
  wallpaper,
  now,
  Masked,
}: {
  lock: ShellLock;
  wallpaper: ImageSourcePropType;
  now: Date;
  /** The platform's masked view, for the clock's material (LockClock). */
  Masked?: ComponentType<MaskedProps>;
}) {
  const name = lock.user.fullName || lock.user.name;
  return (
    <LockBackdrop testID="shell-lock" wallpaper={wallpaper} now={now} Masked={Masked}>
      <UserPicture name={name} />
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
}
