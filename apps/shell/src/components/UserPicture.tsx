import React from "react";
import { Image } from "react-native";
import { Box } from "../primitives";
import { OxyLogo } from "./OxyLogo";

/** The picture of an account without one of its own: the Oxy mark, light on grey. */
const DEFAULT_BACKGROUND = "#767576";
const DEFAULT_MARK = "#c9c9cd";

/**
 * A person's picture on the lock and login screens: their own when they have one, else the
 * system's default picture. It is decorative: the name beside it says who it is.
 */
export function UserPicture({
  picture,
  size = 64,
}: {
  /** The picture's file, as the platform gives it (`file://...`). */
  picture?: string | null;
  size?: number;
}) {
  return (
    <Box
      className="items-center justify-center overflow-hidden"
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: DEFAULT_BACKGROUND,
      }}
    >
      {picture ? (
        <Image source={{ uri: picture }} style={{ width: size, height: size }} />
      ) : (
        <OxyLogo width={size * 0.58} height={size * 0.58} fill={DEFAULT_MARK} />
      )}
    </Box>
  );
}
