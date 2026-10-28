import { styled } from "react-native-css";
import {
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
