import React from "react";
import { Image } from "react-native";
import { SvgXml } from "react-native-svg";
import { Box } from "./primitives";
import type { ShellApplication } from "./model";

export function isSvgIcon(icon: ShellApplication["icon"]): icon is { svg: string } {
  return typeof icon === "object" && icon !== null && "svg" in icon;
}

/**
 * Apple's icon grid, which OxyOS's icons are drawn on: a 1024 canvas whose body, the rounded
 * square, is 896 across at 64 from each edge, its corners 208 (`assets`' and the registry's SVG
 * icons). An icon that has its shape is shown as it is, its body the grid's: one drawn on the grid,
 * and a picture that is its own rounded square, transparent around it (Dropsea's, its corners
 * rounder than the grid's; the platform measures each picture once, `iconBody`). Any other picture
 * (its own square, a round logo, a drawing with holes) is put in that body, clipped to its corners,
 * on white, as macOS 26 puts an icon that is not its shape in one, so every icon in a row is the
 * same size and shape.
 */
const BODY = 896 / 1024;
export const CORNER = 208 / 896;

/**
 * An icon's body alone, `size` across, with nothing added around it: the dock's
 * (docs/adr/0034-the-dock.mdx), as a Mac's Dock shows an icon. An icon drawn on the grid is drawn
 * so that its body is `size`, and so is a picture that is its own shape (`body`, the share of it
 * its body takes: Dropsea's), drawn as it is; any other picture is clipped to the body's corners on
 * nothing, so what shows is the picture's own ground (Atlas's, Inbox's, the Trash's bin on its
 * navy), never a ring or a plate the shell adds.
 */
export function IconBody({
  icon,
  body,
  size,
}: {
  icon: ShellApplication["icon"];
  body?: number;
  size: number;
}) {
  if (isSvgIcon(icon)) {
    // The grid's canvas, its margins outside the body's box.
    const canvas = size / BODY;
    const margin = (size - canvas) / 2;
    return (
      <Box style={{ width: size, height: size }}>
        <Box style={{ position: "absolute", left: margin, top: margin }}>
          <SvgXml xml={icon.svg} width={canvas} height={canvas} />
        </Box>
      </Box>
    );
  }
  if (body) {
    // Its own shape: the picture as it is, nothing clipped or added, its body `size`.
    const picture = size / body;
    const margin = (size - picture) / 2;
    return (
      <Box style={{ width: size, height: size }}>
        <Image
          source={icon}
          resizeMode="contain"
          style={{ position: "absolute", left: margin, top: margin, width: picture, height: picture }}
        />
      </Box>
    );
  }
  // The picture rounds its own corners: a parent's do not clip what it draws on every platform.
  return (
    <Image
      source={icon}
      resizeMode="cover"
      style={{ width: size, height: size, borderRadius: size * CORNER }}
    />
  );
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
  if (app.iconBody) {
    // Its own shape, as an icon drawn on the grid: as it is, no plate under it.
    return (
      <Box className="items-center justify-center" style={{ width: size, height: size }}>
        <IconBody icon={app.icon} body={app.iconBody} size={body} />
      </Box>
    );
  }
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
