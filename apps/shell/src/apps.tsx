import React from "react";
import { Image } from "react-native";
import { SvgXml } from "react-native-svg";
import { Box } from "./primitives";
import type { ShellApplication } from "./model";

function isSvgIcon(icon: ShellApplication["icon"]): icon is { svg: string } {
  return typeof icon === "object" && icon !== null && "svg" in icon;
}

/**
 * Apple's icon grid, which OxyOS's icons are drawn on: a 1024 canvas whose body, the rounded
 * square, is 896 across at 64 from each edge, its corners 208 (`assets`' and the registry's SVG
 * icons). An icon drawn on it is shown as it is; a picture without it (a bitmap, its own square
 * or a round logo) is put in that body, clipped to its corners, on white, as macOS 26 puts an icon
 * that is not its shape in one, so every icon in a row is the same size and shape.
 */
const BODY = 896 / 1024;
const CORNER = 208 / 896;

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
  if (circular) {
    return (
      <Box
        className="items-center justify-center"
        style={{
          width: size,
          height: size,
          borderRadius: size / 2,
          backgroundColor: "#fff",
          overflow: "hidden",
          borderWidth: 1,
          borderColor: "#e5e9ee",
        }}
      >
        {isSvgIcon(app.icon) ? (
          <SvgXml xml={app.icon.svg} width={size - 4} height={size - 4} />
        ) : (
          <Image
            source={app.icon}
            resizeMode="contain"
            style={{ width: size - 4, height: size - 4 }}
          />
        )}
      </Box>
    );
  }
  if (isSvgIcon(app.icon)) {
    // Drawn on the grid: the canvas is the icon, its body and corners its own.
    return (
      <Box style={{ width: size, height: size }}>
        <SvgXml xml={app.icon.svg} width={size} height={size} />
      </Box>
    );
  }
  const body = size * BODY;
  const radius = body * CORNER;
  return (
    <Box
      className="items-center justify-center"
      style={{ width: size, height: size }}
    >
      <Box
        style={{
          width: body,
          height: body,
          borderRadius: radius,
          backgroundColor: "#fff",
          boxShadow: "0 1px 2px #00000030",
        }}
      >
        {/* The picture rounds its own corners: a parent's do not clip what it draws on every
            platform. */}
        <Image
          source={app.icon}
          resizeMode="cover"
          style={{ width: body, height: body, borderRadius: radius }}
        />
      </Box>
    </Box>
  );
}
