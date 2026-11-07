/**
 * The desktop's picture (docs/adr/0048-wallpapers.mdx): what the session's choice draws, as the
 * platform gives it (the Bloom host's `oxyos_wallpaper_show`), and how a change crossfades. The
 * layers are kept here, apart from the component, so the rule is tested on its own.
 */
import type { ImageSourcePropType } from "react-native";

/** How a picture fits the screen: a Mac's Fill Screen, Fit to Screen, Stretch to Fill Screen,
    Centre Image and Tile. */
export type WallpaperFit = "fill" | "fit" | "stretch" | "center" | "tile";

/** What the wallpaper shows: a picture over a colour (a colour alone has no `source`). `key` is
    the identity of what is drawn: the same key is the same picture. */
export type ShellWallpaper = {
  key: string;
  source?: ImageSourcePropType;
  fit: WallpaperFit;
  /** `#rrggbb`: what a colour choice is, and what `fit`, `center` and `tile` leave uncovered. */
  color: string;
  /** The clock brought it (a variant of the sun's, a shuffle's next): a slower crossfade. */
  slow?: boolean;
};

/** A change fades the new picture in over the old once it is drawn; the clock's more slowly. */
export const CROSSFADE_MS = 600;
export const SLOW_CROSSFADE_MS = 2000;

/** React Native's resize mode for a fit. `center` and `tile` keep the picture's size unless it is
    larger than the screen, when it is drawn within it: no picture is decoded larger than its
    display (decision 6). */
export function resizeModeOf(fit: WallpaperFit): "cover" | "contain" | "stretch" | "center" | "repeat" {
  switch (fit) {
    case "fit":
      return "contain";
    case "stretch":
      return "stretch";
    case "center":
      return "center";
    case "tile":
      return "repeat";
    default:
      return "cover";
  }
}

function sourceKey(source: ImageSourcePropType | undefined): unknown {
  if (source && typeof source === "object" && !Array.isArray(source) && "uri" in source) return source.uri;
  return source;
}

/** Whether two wallpapers draw the same: the same picture, fit and colour. */
export function sameWallpaper(a: ShellWallpaper, b: ShellWallpaper): boolean {
  return a.key === b.key && a.fit === b.fit && a.color === b.color && sourceKey(a.source) === sourceKey(b.source);
}

/** A picture drawn, oldest first; the last is the newest. `shown` once it has faded in. */
export type WallpaperLayer = { id: number; wallpaper: ShellWallpaper; shown: boolean };

/** The layers with `wallpaper` coming in over them, numbered `id`: the first one is shown at
    once; one that draws the same as the newest is no change. */
export function withIncoming(layers: readonly WallpaperLayer[], wallpaper: ShellWallpaper, id: number): WallpaperLayer[] {
  const newest = layers[layers.length - 1];
  if (newest && sameWallpaper(newest.wallpaper, wallpaper)) return layers as WallpaperLayer[];
  return [...layers, { id, wallpaper, shown: layers.length === 0 }];
}

/** The layers once layer `id` has faded in: those beneath it go, as nothing of them shows. */
export function shownLayers(layers: readonly WallpaperLayer[], id: number): WallpaperLayer[] {
  const at = layers.findIndex((layer) => layer.id === id);
  if (at < 0) return layers as WallpaperLayer[];
  return layers.slice(at).map((layer, index) => (index === 0 ? { ...layer, shown: true } : layer));
}

/** How long layer `wallpaper` takes to fade in. */
export function crossfadeMs(wallpaper: ShellWallpaper): number {
  return wallpaper.slow ? SLOW_CROSSFADE_MS : CROSSFADE_MS;
}
