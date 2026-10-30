import React from "react";
import { Image } from "react-native";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]!.toUpperCase())
    .join("");

/**
 * A person's picture on the lock and login screens: their own when they have one, else their
 * initials. It is decorative: the name beside it says who it is.
 */
export function UserPicture({
  name,
  picture,
  size = 64,
}: {
  name: string;
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
        backgroundColor: "rgba(255,255,255,0.32)",
      }}
    >
      {picture ? (
        <Image source={{ uri: picture }} style={{ width: size, height: size }} />
      ) : (
        <Text style={{ color: "#fff", fontSize: size * 0.375, fontWeight: "600" }}>
          {initials(name)}
        </Text>
      )}
    </Box>
  );
}
