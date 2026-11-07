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
      (docs/adr/0011-transitions.mdx). A candidates region is the candidate window
      (docs/adr/0041-input-methods.mdx): its surface is as large as the region, placed by the
      platform at the caret of the field being typed in, and never takes the keyboard. */
  kind: "panel" | "overlay" | "backdrop" | "notification" | "lock" | "candidates";
  edge?: "top" | "bottom" | "left" | "right";
  /** Whether a panel keeps its strip of the output out of where windows are placed. */
  exclusive?: boolean;
  /** Whether the region asks for the keyboard (the top bar driven from the keyboard): the
      platform gives it, citing the person's latest input. */
  keyboard?: boolean;
  /** Where the keyboard's focus goes once the region has the keyboard: "first", its first
      control, or a control's testID (docs/adr/0039-keyboard-access.mdx). */
  focusTarget?: string;
  /** Each new value puts the keyboard's focus on `focusTarget` again. */
  focusRequest?: number;
  /** The keyboard, once the region had it, went elsewhere (a press in a window). */
  onKeyboardLost?: () => void;
  /** Whether no screenshot captures the region (the screenshot interface itself, its thumbnail,
      docs/adr/0024-system-shortcuts.mdx, decision 10). */
  excludeFromCapture?: boolean;
  /** Whether the windows the region shows live (`WindowView`) are the windows themselves, moved
      there, as App Exposé's (docs/adr/0045-app-expose.mdx): while it shows, the platform draws each
      in its view alone, flying there from where it is as the region arrives and back as it goes.
      Without it, a view is a copy of its window, which stays where it is. */
  movesWindows?: boolean;
  /** Which view a touchpad's swipe drives the region is, Mission Control's or App Exposé's
      (docs/adr/0046-interactive-gestures.mdx): while three or four fingers open or close it, the
      platform draws it, its moved windows with it, as far as they have it. */
  gesture?: "mission-control" | "app-expose";
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
/** A window's size, in the proportions its overview card is drawn in (ADR 0022). */
export type WindowSize = { width: number; height: number };

export type WindowViewProps = {
  windowId: string;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
};

/**
 * Where windows minimize into (docs/adr/0011-transitions.mdx, the genie): the platform's
 * compositor has each window `windowIds` names go into the view's box when it is minimized and
 * come back out of it when it is restored, as a Mac's Dock tells the window server where each
 * window's icon is. The view lays out and draws `children` as any view; `style` sizes it.
 */
export type MinimizeTargetProps = {
  windowIds: readonly string[];
  style?: StyleProp<ViewStyle>;
  children?: ReactNode;
};

/** A scroll the views inside a `ScrollTarget` did not take: (dx, dy) as the content would move
    (dy below 0 scrolls up, as a wheel turned away from the person), whether a touchpad's fingers
    made it, and, once they lift, its end, which moves nothing. */
export type ShellScrollEvent = { dx: number; dy: number; finger: boolean; end: boolean };

/**
 * What takes the scrolls nothing within it takes (the dock's icons, whose scroll up opens App
 * Exposé as a Mac's Dock can, docs/adr/0045-app-expose.mdx), as an Android view takes the scroll
 * events its children left. Laid out and drawn as a view; `style` sizes it.
 */
export type ScrollTargetProps = {
  onScroll: (event: ShellScrollEvent) => void;
  style?: StyleProp<ViewStyle>;
  children?: ReactNode;
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

/** A part of a query the person has made a token of (an application, a kind, a date), and one
    result, as the search index takes and gives them (docs/adr/0051-search.mdx): Bloom's
    PlatformSearch says their fields; the shell reads only a result's `id`. */
export type ShellSearchToken = { [field: string]: unknown };
export type ShellSearchItem = { id: string; [field: string]: unknown };

export type ShellSearchQuery = {
  text: string;
  tokens?: ShellSearchToken[];
  within?: string[];
  categories?: string[];
  limit?: number;
  language?: string;
};

/**
 * The session's search index (docs/adr/0051-search.mdx), Spotlight's: a query's results, its top
 * hit and whether the index is still being built; a listener told when what was found may have
 * changed; the person's choice of a result, which ranks it higher; the items an application gives
 * the index; and, for Settings alone, the index's preferences and status. A refusal rejects with an
 * Error whose `code` is the reason.
 */
export type ShellSearch = {
  query(query: ShellSearchQuery): Promise<{ items: ShellSearchItem[]; topHit: string | null; indexing: boolean }>;
  watch(listener: () => void): () => void;
  used(id: string): void;
  index: {
    donate(items: ShellSearchItem[]): Promise<void>;
    remove(ids: string[]): Promise<void>;
    removeAll(): Promise<void>;
  };
  preferences(): Promise<object>;
  setPreferences(preferences: object): Promise<object>;
  status(): Promise<object>;
};

export type ShellPlatform = {
  Surface: ComponentType<ShellSurfaceProps>;
  /** Where the platform masks views; without it, what would be masked is drawn plainly. */
  Masked?: ComponentType<MaskedProps>;
  /** Where the platform shows windows live (the overview's cards); without it, a window's
      `thumbnail` if it has one, or its icon. */
  WindowView?: ComponentType<WindowViewProps>;
  /** Where the platform minimizes windows into a view (the dock's icon of their application);
      without it, the dock's icons are only pictures and the platform minimizes as it does. */
  MinimizeTarget?: ComponentType<MinimizeTargetProps>;
  /** Where the platform hands a view the scrolls nothing within it took (the dock's icons);
      without it, the dock's icons take no scroll. */
  ScrollTarget?: ComponentType<ScrollTargetProps>;
  /** The windows' sizes now, by window id, where the platform keeps them apart from the snapshot
      (the session's shell): read by the overview alone, so that a window resized or zoomed renders
      the overview, when it is open, and nothing else of the shell. Without it, each window's
      `thumbnailSize`. */
  useWindowSizes?: () => ReadonlyMap<string, WindowSize> | undefined;
  /** Whether a window zoomed to fill the work area, or full screen, covers this display now, where
      the platform says (the session's shell): read by the dock's shelf alone, whose top corners are
      square while one does (docs/adr/0034-the-dock.mdx), so that a window zoomed or restored renders
      the shelf and nothing else of the shell. Without it, the corners stay round. */
  useDisplayCovered?: () => boolean;
  /** Says `text` to assistive technologies, politely (after what they are saying) or at once;
      without it, React Native's announcement, which interrupts. */
  announce?: (text: string, polite: boolean) => void;
  /** Its keys before the focused field, where it offers them; elsewhere a field's own
      `onKeyPress` is all there is. */
  keyboard?: ShellKeyboard;
  /** Asks the keyboard for the desktop (docs/adr/0047-finder-drags-and-the-desktop.mdx, decision
      5), citing the press on it, as a press on a Mac's desktop makes Finder active: the platform
      gives it only for that press. Without it, the desktop never has the keyboard. */
  focusDesktop?: () => void;
  /** Whether the desktop has the keyboard now, where the platform says: read by the desktop alone,
      whose selection is drawn in the accent's colour only then. */
  useDesktopKeyboard?: () => boolean;
  /** The session's search index (docs/adr/0051-search.mdx), where the platform has one: Spotlight
      queries it. */
  search?: ShellSearch;
};

/** Everywhere but OxyOS (the browser preview): every region stays in the one surface. */
export const inPlace: ShellPlatform = {
  Surface: ({ children }) => <>{children}</>,
};
