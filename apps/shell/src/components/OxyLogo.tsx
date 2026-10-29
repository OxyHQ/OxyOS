import React from "react";
import Svg, { Path } from "react-native-svg";
import { SOCIAL_PROVIDERS } from "@oxy.so/bloom/social-button";

const mark = SOCIAL_PROVIDERS.oxy;

/**
 * The Oxy mark, in one colour, as the system's own sign: the menu bar's first item, as the
 * Apple menu is. Bloom's drawing (the one the Oxy SDK ships), never redrawn here. An icon as
 * Bloom's components take one (`width`, `height`, `fill`).
 */
export function OxyLogo({
  width = 16,
  height = 16,
  fill = "#fff",
}: {
  width?: number;
  height?: number;
  fill?: string;
}) {
  return (
    <Svg width={width} height={height} viewBox={mark.viewBox} aria-hidden>
      <Path d={mark.path} fill={fill} />
    </Svg>
  );
}
