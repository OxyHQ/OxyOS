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

/**
 * `children` drawn only where `mask` is opaque, as React Native's MaskedView (the lock screen's
 * clock is the wallpaper, blurred, seen through its digits). `style` sizes the view; the mask
 * fills it.
 */
export type MaskedProps = {
  mask: ReactNode;
  style?: StyleProp<ViewStyle>;
  children: ReactNode;
};

/**
 * A window's live contents where the view is (docs/adr/0022-window-views.mdx): the platform's
 * compositor draws the window there itself, fitted to `style`'s size in its proportions, clipped
 * as the view is, with the view's top-left corner radius. The shell never holds the pixels.
 */
export type WindowViewProps = {
  windowId: string;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
};

export type ShellPlatform = {
  Surface: ComponentType<ShellSurfaceProps>;
  /** Where the platform masks views; without it, what would be masked is drawn plainly. */
  Masked?: ComponentType<MaskedProps>;
  /** Where the platform shows windows live (the overview's cards); without it, a window's
      `thumbnail` if it has one, or its icon. */
  WindowView?: ComponentType<WindowViewProps>;
};

/** Everywhere but OxyOS (the browser preview): every region stays in the one surface. */
export const inPlace: ShellPlatform = {
  Surface: ({ children }) => <>{children}</>,
};
