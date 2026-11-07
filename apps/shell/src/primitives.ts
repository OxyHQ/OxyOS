import { styled } from "react-native-css";
import {
  AccessibilityInfo,
  View,
  Pressable,
  type PressableProps,
  ScrollView,
  type ViewProps,
  type ScrollViewProps,
} from "react-native";
import type { ComponentType, RefAttributes, ComponentRef } from "react";

// The same explicit NativeWind v5 interop used by Bloom's primitives.
export const Box = styled(View) as ComponentType<
  ViewProps & RefAttributes<ComponentRef<typeof View>> & { className?: string }
>;
type ShellScroll = ComponentType<
  ScrollViewProps &
    RefAttributes<ComponentRef<typeof ScrollView>> & {
      className?: string;
      contentContainerClassName?: string;
    }
>;
// Limit the mapping's targets; expanding every nested RN ScrollView prop exceeds
// TypeScript's dot-path union budget. Public props retain their full RN types.
const styleScroll = styled as (
  component: typeof ScrollView,
  mapping: {
    className: "style";
    contentContainerClassName: "contentContainerStyle";
  },
) => ShellScroll;
export const Scroll = styleScroll(ScrollView, {
  className: "style",
  contentContainerClassName: "contentContainerStyle",
});

/** Unpainted hit region for shell backdrops; unlike a button it has no hover material. */
export const PressArea = styled(Pressable) as ComponentType<
  PressableProps & { className?: string }
>;

/**
 * Moves the screen reader's focus to `view` (React Native's accessibility focus), where the
 * platform has it: React Native for the web has no such event, and the browser's own focus is all
 * there is there.
 */
export function focusForAccessibility(view: ComponentRef<typeof View> | null | undefined) {
  if (!view || typeof AccessibilityInfo.sendAccessibilityEvent !== "function") return;
  AccessibilityInfo.sendAccessibilityEvent(
    view as Parameters<typeof AccessibilityInfo.sendAccessibilityEvent>[0],
    "focus",
  );
}
