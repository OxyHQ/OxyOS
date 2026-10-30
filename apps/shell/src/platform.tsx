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
  /** A lock region is the lock screen: all that shows while the session is locked. A
      notification is above everything but the lock: a hardware key's bezel takes no input
      (docs/adr/0025-hardware-keys-and-control-center.mdx); one at the right edge is a banner, at
      the output's top-right corner, which takes presses (docs/adr/0026-notifications.mdx). A
      backdrop is an overlay's, beneath it (Launchpad's blurred wallpaper): the compositor fades
      it in and out and never scales it, while the overlay above settles
      (docs/adr/0011-transitions.mdx). */
  kind: "panel" | "overlay" | "backdrop" | "notification" | "lock";
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

/** A key press as the platform offers it before the focused field (W3C names). */
export type ShellKeyEvent = {
  key: string;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
};

/**
 * The platform's key presses before the focused field receives them: a listener returning true
 * took the key, and the field does not receive it (Launchpad's arrows and Return while its search
 * has the keyboard). Returns the unsubscribe.
 */
export type ShellKeyboard = {
  subscribe(listener: (event: ShellKeyEvent) => boolean): () => void;
};

export type ShellPlatform = {
  Surface: ComponentType<ShellSurfaceProps>;
  /** Where the platform masks views; without it, what would be masked is drawn plainly. */
  Masked?: ComponentType<MaskedProps>;
  /** Where the platform shows windows live (the overview's cards); without it, a window's
      `thumbnail` if it has one, or its icon. */
  WindowView?: ComponentType<WindowViewProps>;
  /** Says `text` to assistive technologies, politely (after what they are saying) or at once;
      without it, React Native's announcement, which interrupts. */
  announce?: (text: string, polite: boolean) => void;
  /** Its keys before the focused field, where it offers them; elsewhere a field's own
      `onKeyPress` is all there is. */
  keyboard?: ShellKeyboard;
};

/** Everywhere but OxyOS (the browser preview): every region stays in the one surface. */
export const inPlace: ShellPlatform = {
  Surface: ({ children }) => <>{children}</>,
};
