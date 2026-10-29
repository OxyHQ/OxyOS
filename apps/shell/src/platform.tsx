import React from "react";
import type { ComponentType, ReactNode } from "react";
import type { StyleProp, ViewStyle } from "react-native";

/**
 * Where the shell's regions are drawn. On OxyOS the top bar and the dock are panels at the
 * output's edges and the open overview or launcher an overlay over everything: separate
 * compositor surfaces, while the shell stays one React tree (docs/adr/0009-session-shell.mdx).
 * Each region keeps its place in the shell's layout; `style` is what its wrapper must fill.
 */
export type ShellSurfaceProps = {
  /** A lock region is the lock screen: all that shows while the session is locked. */
  kind: "panel" | "overlay" | "lock";
  edge?: "top" | "bottom" | "left" | "right";
  /** Whether a panel keeps its strip of the output out of where windows are placed. */
  exclusive?: boolean;
  style?: StyleProp<ViewStyle>;
  children: ReactNode;
};

export type ShellPlatform = {
  Surface: ComponentType<ShellSurfaceProps>;
};

/** Everywhere but OxyOS (the browser preview): every region stays in the one surface. */
export const inPlace: ShellPlatform = {
  Surface: ({ children }) => <>{children}</>,
};
