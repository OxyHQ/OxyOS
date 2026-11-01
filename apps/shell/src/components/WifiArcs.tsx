import React from "react";
import Svg, { Circle, Path } from "react-native-svg";

/**
 * Wi-Fi's signal in arcs, as a Mac draws it: the arcs its signal fills in `color`, the others in
 * `faint`, and the dot lit when `dot`. Bloom's icons have Wi-Fi's whole glyph but no level of it,
 * so the top bar and the Control Center draw the levels with this one shape.
 */
export function WifiArcs({
  arcs,
  dot = true,
  color,
  faint,
  width = 14,
  height = 12,
}: {
  /** 0 to 3. */
  arcs: number;
  dot?: boolean;
  color: string;
  faint: string;
  width?: number;
  height?: number;
}) {
  const arc = (lit: boolean) => (lit ? color : faint);
  return (
    <Svg width={width} height={height} viewBox="0 0 20 16">
      <Path d="M1 5.6a13 13 0 0 1 18 0" stroke={arc(arcs >= 3)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M4.2 8.9a8.4 8.4 0 0 1 11.6 0" stroke={arc(arcs >= 2)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M7.4 12.1a3.9 3.9 0 0 1 5.2 0" stroke={arc(arcs >= 1)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Circle cx="10" cy="14.6" r="1.3" fill={arc(dot)} />
    </Svg>
  );
}
