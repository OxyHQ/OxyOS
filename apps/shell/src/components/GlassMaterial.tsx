/**
 * The shell's glass on the platform's material (docs/adr/0069-the-platforms-material.mdx, decision
 * 7): where the platform has Bloom's `material` capability, a panel's glass is Bloom's floating
 * surface, the platform's material (what is behind the panel, the windows and the desktop, blurred
 * and saturated) with the panel's fill over it as its tint (`materialTint`). Elsewhere, and with
 * Reduce transparency on, it is the fill as `glass` draws it, translucent or opaque. The panel draws
 * no background of its own while the material is under it (`useGlass`), and `GlassMaterial`, its
 * first child, draws both in its shape.
 */
import React, { memo } from "react";
import { View, type ViewStyle } from "react-native";
import { usePlatformMaterial } from "@oxy.so/bloom/platform-surface";
import { useReducedTransparency, useTheme } from "@oxy.so/bloom/theme";
import { glass, materialTint } from "./glass";

/** Whether a panel's glass is the platform's material now, and the background the panel draws. */
export function useGlass(fill: string): { material: boolean; backgroundColor: string } {
  const Material = usePlatformMaterial();
  const reduced = useReducedTransparency();
  const material = Material !== null && !reduced;
  return { material, backgroundColor: material ? "transparent" : glass(fill, reduced) };
}

const FILL = { position: "absolute", top: 0, right: 0, bottom: 0, left: 0 } as const;

/**
 * The platform's material and the fill's tint over it, filling the panel within `shape` (its corner
 * radii, less any border it draws inside them; none for a panel that clips them to its own corners,
 * whose shape they then take); nothing where `useGlass` says the panel draws its own fill.
 */
export const GlassMaterial = memo(function GlassMaterial({
  fill,
  shape,
  testID,
}: {
  fill: string;
  shape?: ViewStyle;
  testID?: string;
}) {
  const Material = usePlatformMaterial();
  const reduced = useReducedTransparency();
  const { isDark } = useTheme();
  if (Material === null || reduced) return null;
  return (
    <>
      <Material testID={testID} appearance={isDark ? "dark" : "light"} style={[FILL, shape]} />
      <View pointerEvents="none" style={[FILL, shape, { backgroundColor: materialTint(fill) }]} />
    </>
  );
});
