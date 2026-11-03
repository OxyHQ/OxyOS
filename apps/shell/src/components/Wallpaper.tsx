import React, { memo } from "react";
import { Image, type ImageSourcePropType } from "react-native";
import { useShellMessages } from "../useShellMessages";

/**
 * The wallpaper. On the desktop it is an image an assistive technology names; behind the
 * launcher and the lock screen it is `decorative`, and is not read.
 */
export const Wallpaper = memo(function Wallpaper({
  source,
  decorative = false,
}: {
  source: ImageSourcePropType;
  decorative?: boolean;
}) {
  const messages = useShellMessages();
  return (
    <Image
      source={source}
      resizeMode="cover"
      accessibilityRole={decorative ? undefined : "image"}
      accessibilityLabel={decorative ? undefined : messages.wallpaper}
      style={{ width: "100%", height: "100%" }}
    />
  );
});
