import React, { memo, useCallback, useEffect, useRef, useState } from "react";
import { Animated, Image, StyleSheet, View } from "react-native";
import { useShellMessages } from "../useShellMessages";
import {
  crossfadeMs,
  resizeModeOf,
  shownLayers,
  withIncoming,
  type ShellWallpaper,
  type WallpaperLayer,
} from "../wallpaper";

/**
 * The wallpaper (docs/adr/0048-wallpapers.mdx): a picture over its colour, as it fits the screen.
 * A change fades the new one in over the old once it has been drawn, and the old one goes then;
 * Reduce Motion keeps the crossfade, as ADR 0011 keeps fades. On the desktop it is an image an
 * assistive technology names; behind the launcher, App Exposé, the lock and login screens it is
 * `decorative`, and is not read.
 */
export const Wallpaper = memo(function Wallpaper({
  wallpaper,
  decorative = false,
}: {
  wallpaper: ShellWallpaper;
  decorative?: boolean;
}) {
  const messages = useShellMessages();
  const next = useRef(1);
  const [layers, setLayers] = useState<WallpaperLayer[]>(() => withIncoming([], wallpaper, 0));
  useEffect(() => {
    setLayers((current) => withIncoming(current, wallpaper, next.current++));
  }, [wallpaper]);
  const shown = useCallback((id: number) => setLayers((current) => shownLayers(current, id)), []);
  return (
    <View
      style={StyleSheet.absoluteFill}
      pointerEvents="none"
      accessible={!decorative}
      accessibilityRole={decorative ? undefined : "image"}
      accessibilityLabel={decorative ? undefined : messages.wallpaper}
    >
      {layers.map((layer) => (
        <Layer key={layer.id} layer={layer} onShown={shown} />
      ))}
    </View>
  );
});

/** One picture: shown at once if it is the first, else faded in once it is drawn (a colour at once,
    a picture once it loaded or failed to, which leaves its colour). */
const Layer = memo(function Layer({ layer, onShown }: { layer: WallpaperLayer; onShown: (id: number) => void }) {
  const { id, wallpaper } = layer;
  const opacity = useRef(new Animated.Value(layer.shown ? 1 : 0)).current;
  const started = useRef(layer.shown);
  const reveal = useCallback(() => {
    if (started.current) return;
    started.current = true;
    Animated.timing(opacity, { toValue: 1, duration: crossfadeMs(wallpaper), useNativeDriver: true }).start(
      ({ finished }) => {
        if (finished) onShown(id);
      },
    );
  }, [id, wallpaper, opacity, onShown]);
  useEffect(() => {
    if (!wallpaper.source) reveal();
  }, []);
  return (
    <Animated.View style={[StyleSheet.absoluteFill, { opacity, backgroundColor: wallpaper.color }]}>
      {wallpaper.source ? (
        <Image
          source={wallpaper.source}
          resizeMode={resizeModeOf(wallpaper.fit)}
          onLoad={reveal}
          onError={reveal}
          style={{ width: "100%", height: "100%" }}
        />
      ) : null}
    </Animated.View>
  );
});
