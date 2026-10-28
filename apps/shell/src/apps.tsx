import React from "react";
import { Image } from "react-native";
import { SvgXml } from "react-native-svg";
import { Box } from "./primitives";
import type { ShellApplication } from "./model";

function isSvgIcon(icon: ShellApplication["icon"]): icon is { svg: string } {
  return typeof icon === "object" && icon !== null && "svg" in icon;
}

/** Actual application artwork supplied by the registry. No generic recoloured glyphs. */
export function AppIcon({
  app,
  size = 44,
  circular = false,
}: {
  app: ShellApplication;
  size?: number;
  circular?: boolean;
}) {
  const artworkSize = circular
    ? size - 4
    : isSvgIcon(app.icon)
      ? size * 0.82
      : size;
  return (
    <Box
      className="items-center justify-center"
      style={{
        width: size,
        height: size,
        borderRadius: circular ? size / 2 : size * 0.22,
        backgroundColor: "#fff",
        overflow: "hidden",
        borderWidth: circular ? 1 : 0,
        boxShadow: circular
          ? undefined
          : "0 2px 3px #0a153b35, 0 0 0 .5px #ffffff25",
        borderColor: "#e5e9ee",
      }}
    >
      {isSvgIcon(app.icon) ? (
        <SvgXml xml={app.icon.svg} width={artworkSize} height={artworkSize} />
      ) : (
        <Image
          source={app.icon}
          resizeMode="contain"
          style={{ width: artworkSize, height: artworkSize }}
        />
      )}
    </Box>
  );
}
