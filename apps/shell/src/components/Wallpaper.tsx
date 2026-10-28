import React from "react";
import { Image, type ImageSourcePropType } from "react-native";

export function Wallpaper({ source }: { source: ImageSourcePropType }) {
  return (
    <Image
      source={source}
      resizeMode="cover"
      accessibilityLabel="Fondo de escritorio"
      style={{ width: "100%", height: "100%" }}
    />
  );
}
