import React from "react";
import { Image } from "react-native";
import { Box } from "../primitives";
import { DEFAULT_USER_PICTURE } from "./default-user-picture";

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
      }}
    >
      <Image
        source={picture ? { uri: picture } : DEFAULT_USER_PICTURE}
        style={{ width: size, height: size, borderRadius: size / 2 }}
      />
    </Box>
  );
}
