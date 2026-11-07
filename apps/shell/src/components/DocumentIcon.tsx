import React from "react";
import { View } from "react-native";
import Svg, { Path } from "react-native-svg";
import type { RiFileLine } from "@oxy.so/bloom/icons/RiFileLine";

/**
 * A document's icon, as Finder draws one it has no picture of: a white page, its corner folded,
 * with its kind's glyph on it (the desktop's files, docs/adr/0047-finder-drags-and-the-desktop.mdx,
 * and Spotlight's results). Drawn on a 64 grid, `size` across.
 */
export function DocumentIcon({ glyph: Glyph, size }: { glyph: typeof RiFileLine; size: number }) {
  const glyph = (size * 24) / 64;
  return (
    <View style={{ width: size, height: size }}>
      <Svg width={size} height={size} viewBox="0 0 64 64">
        <Path
          d="M14 4h26l14 14v40a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z"
          fill="#ffffff"
          stroke="#c9c9cf"
          strokeWidth={1.2}
        />
        <Path d="M40 4v12a2 2 0 0 0 2 2h12" fill="none" stroke="#c9c9cf" strokeWidth={1.2} />
      </Svg>
      <View style={{ position: "absolute", left: 0, right: 0, top: (size * 26) / 64, alignItems: "center" }}>
        <Glyph width={glyph} height={glyph} fill="#6e6e78" />
      </View>
    </View>
  );
}
