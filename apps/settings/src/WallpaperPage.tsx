/**
 * Settings › Wallpaper (docs/adr/0048-wallpapers.mdx, decision 7), as a Mac's System Settings ›
 * Wallpaper: the current wallpaper and its options at the top, Show on all desktops, the system's
 * wallpapers by category (each row a radio group: one keyboard stop whose arrows choose, its
 * Show All and its shuffle), the person's photos and folders, and colours with one of their own.
 * Every choice is one text the session judges and keeps where Settings' window is; the page shows
 * what the session tells it.
 */
import * as React from "react";
import { Image, PixelRatio, Pressable, useColorScheme, View, type ImageResizeMode } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Dialog } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { RiFolder3Line, RiImageAddLine, RiPaletteLine, RiShuffleLine, RiSubtractLine } from "@oxy.so/bloom/icons";
import { RadioGroup, type RadioOption } from "@oxy.so/bloom/radio";
import { SettingsSelect } from "@oxy.so/bloom/settings-modal";
import { Slider } from "@oxy.so/bloom/slider";
import { Switch } from "@oxy.so/bloom/switch";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import type { SettingsActions, SettingsSnapshot, WallpaperActions } from "./model";
import type { Messages } from "./messages";
import {
  addedWithChosen,
  baseName,
  chooseTile,
  choiceText,
  COLORS_CATEGORY,
  colorsWithChosen,
  fileUri,
  findWallpaper,
  FITS,
  hexToHsv,
  hsvToHex,
  normalizeHex,
  SHUFFLE_EVERY,
  shownChoice,
  shownTiles,
  tileValue,
  withAdded,
  type AddedWallpaper,
  type ShuffleEvery,
  type SystemWallpaper,
  type WallpaperCatalog,
  type WallpaperChoice,
  type WallpaperFit,
} from "./wallpaper";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages };
type Words = Messages["wallpaper"];

/** A row's tile, 16:10 as the displays it stands for, and the top's preview. */
const TILE = { width: 104, height: 65 } as const;
const PREVIEW = { width: 176, height: 110 } as const;
const RADIUS = 8;
/** The variant menu's Automatic: no variant's id (an id begins with a letter). */
const AUTOMATIC = "-automatic";
/** The selection ring: its width and its gap from the picture, as a Mac's. */
const RING = 2.5;
const GAP = 2;
const PAGE = { gap: 24 } as const;
const ROW = { flexDirection: "row", flexWrap: "wrap", gap: 10 } as const;
const BUTTONS = { flexDirection: "row", flexWrap: "wrap", gap: 8, marginTop: 4 } as const;
/** A tile's own radio: none of the chip's pill, the tile draws itself. */
const TILE_RADIO = {
  padding: 0,
  paddingLeft: 0,
  paddingRight: 0,
  paddingTop: 0,
  paddingBottom: 0,
  borderWidth: 0,
  minHeight: 0,
  minWidth: 0,
  borderRadius: RADIUS + GAP + RING,
  backgroundColor: "transparent",
} as const;

/** Thumbnails already made, by what was asked: a page shown again draws them at once. */
const thumbnails = new Map<string, string | null>();
const folders = new Map<string, string[]>();

/** The picture to draw for `uri` at `width` × `height` points: the platform's cached thumbnail at
    the display's pixels, or the picture itself where the platform has no cache. */
function useThumbnail(
  platform: WallpaperActions | undefined,
  uri: string | null,
  width: number,
  height: number,
): string | null {
  const pixels = PixelRatio.get();
  const w = Math.round(width * pixels);
  const h = Math.round(height * pixels);
  const key = uri ? `${uri}|${w}x${h}` : "";
  const make = platform?.thumbnail;
  const [shown, setShown] = React.useState<string | null>(() =>
    !uri ? null : make ? (thumbnails.get(key) ?? null) : uri,
  );
  React.useEffect(() => {
    if (!uri) return setShown(null);
    if (!make) return setShown(uri);
    if (thumbnails.has(key)) return setShown(thumbnails.get(key) ?? null);
    let live = true;
    make(uri, w, h).then(
      (made) => {
        thumbnails.set(key, made);
        if (live) setShown(made);
      },
      () => live && setShown(null),
    );
    return () => {
      live = false;
    };
  }, [key, make]);
  return shown;
}

/** A folder's images, as the platform lists them (none where it cannot). */
function useFolder(platform: WallpaperActions | undefined, path: string | null): string[] {
  const list = platform?.folder;
  const [images, setImages] = React.useState<string[]>(() => (path ? (folders.get(path) ?? []) : []));
  React.useEffect(() => {
    if (!path || !list) return setImages([]);
    let live = true;
    list(path).then(
      (found) => {
        folders.set(path, found);
        if (live) setImages(found);
      },
      () => live && setImages([]),
    );
    return () => {
      live = false;
    };
  }, [path, list]);
  return images;
}

/** A system wallpaper's picture as it looks now: its fixed variant's, the appearance's for one
    that follows it, else its preview. */
function wallpaperPicture(wallpaper: SystemWallpaper, variant: string | null, dark: boolean): string {
  const id = variant ?? (wallpaper.automatic === "appearance" ? (dark ? "dark" : "light") : null);
  return wallpaper.variants.find((v) => v.id === id)?.preview ?? wallpaper.preview;
}

/** How a photo's fit draws in a preview. */
const FIT_MODE: Record<WallpaperFit, ImageResizeMode> = {
  fill: "cover",
  fit: "contain",
  stretch: "stretch",
  center: "center",
  tile: "repeat",
};

/** A picture in a tile's rounded frame: an image, a colour, or several in a mosaic. Decorative:
    the radio around it is named. */
function Picture({
  platform,
  uri,
  color,
  fit = "fill",
  size,
  children,
}: {
  platform: WallpaperActions | undefined;
  uri?: string | null;
  color?: string;
  fit?: WallpaperFit;
  size: { width: number; height: number };
  children?: React.ReactNode;
}) {
  const theme = useTheme();
  // A picture centred or tiled is drawn smaller than the tile, as it is smaller than a display.
  const small = fit === "center" || fit === "tile";
  const shown = useThumbnail(
    platform,
    uri ?? null,
    small ? size.width / 3 : size.width,
    small ? size.height / 3 : size.height,
  );
  return (
    <View
      style={{
        width: size.width,
        height: size.height,
        borderRadius: RADIUS,
        overflow: "hidden",
        backgroundColor: color ? `#${color}` : theme.colors.borderLight,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      {shown ? (
        <Image
          source={{ uri: shown }}
          resizeMode={FIT_MODE[fit]}
          style={
            fit === "center"
              ? { width: size.width / 3, height: size.height / 3 }
              : { position: "absolute", top: 0, left: 0, width: size.width, height: size.height }
          }
        />
      ) : null}
      {children}
    </View>
  );
}

/** A shuffle's picture: up to four of what it shuffles, side by side as many as there are (two
    halves, one beside two, or four quarters), and its glyph. */
function Mosaic({
  platform,
  uris,
  colors,
  size,
}: {
  platform: WallpaperActions | undefined;
  uris: readonly string[];
  colors: readonly string[];
  size: { width: number; height: number };
}) {
  const theme = useTheme();
  const cells: { uri?: string; color?: string }[] = (
    uris.length > 0 ? uris.map((uri) => ({ uri })) : colors.map((color) => ({ color }))
  ).slice(0, 4);
  const { width, height } = size;
  // Each cell's box, by how many there are.
  const boxes: { left: number; top: number; width: number; height: number }[] =
    cells.length <= 1
      ? [{ left: 0, top: 0, width, height }]
      : cells.length === 2
        ? [
            { left: 0, top: 0, width: width / 2, height },
            { left: width / 2, top: 0, width: width / 2, height },
          ]
        : cells.length === 3
          ? [
              { left: 0, top: 0, width: width / 2, height },
              { left: width / 2, top: 0, width: width / 2, height: height / 2 },
              { left: width / 2, top: height / 2, width: width / 2, height: height / 2 },
            ]
          : [0, 1, 2, 3].map((i) => ({
              left: (i % 2) * (width / 2),
              top: Math.floor(i / 2) * (height / 2),
              width: width / 2,
              height: height / 2,
            }));
  return (
    <View style={{ width, height, borderRadius: RADIUS, overflow: "hidden", backgroundColor: theme.colors.borderLight }}>
      {cells.map((cell, i) => (
        <MosaicCell key={i} platform={platform} cell={cell} box={boxes[i]!} />
      ))}
      <View
        style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, alignItems: "center", justifyContent: "center" }}
      >
        <View
          style={{
            width: 28,
            height: 28,
            borderRadius: 14,
            backgroundColor: "rgba(0,0,0,0.5)",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <RiShuffleLine width={16} fill="#ffffff" />
        </View>
      </View>
    </View>
  );
}

function MosaicCell({
  platform,
  cell,
  box,
}: {
  platform: WallpaperActions | undefined;
  cell: { uri?: string; color?: string };
  box: { left: number; top: number; width: number; height: number };
}) {
  const shown = useThumbnail(platform, cell.uri ?? null, box.width, box.height);
  return (
    <View style={{ position: "absolute", ...box, backgroundColor: cell.color ? `#${cell.color}` : undefined }}>
      {shown ? <Image source={{ uri: shown }} resizeMode="cover" style={{ width: box.width, height: box.height }} /> : null}
    </View>
  );
}

/** A tile's frame: the accent's ring around the chosen one, with a gap, as a Mac outlines it. */
function Frame({ chosen, children }: { chosen: boolean; children: React.ReactNode }) {
  const theme = useTheme();
  return (
    <View
      style={{
        padding: GAP,
        borderRadius: RADIUS + GAP + RING,
        borderWidth: RING,
        borderColor: chosen ? theme.colors.primary : "transparent",
      }}
    >
      {children}
    </View>
  );
}

/** A section's heading as Bloom's grouped sections draw theirs, with an action at its end (*Show
    All*, as a Mac's Wallpaper settings place it). */
function SectionHeading({ title, children }: { title: string; children?: React.ReactNode }) {
  const theme = useTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        paddingLeft: 20,
        paddingRight: 8,
        minHeight: 28,
        marginBottom: 2,
      }}
    >
      <Text
        variant="caption-2-semibold"
        accessibilityRole="header"
        style={{ color: theme.colors.textSecondary, textTransform: "uppercase", letterSpacing: 0.6 }}
      >
        {title}
      </Text>
      {children}
    </View>
  );
}

/** One tile of a row: what it chooses, its name, and its picture. */
type Tile = { value: string; label: string; picture: React.ReactNode; testID: string };

/** A row of tiles, one radio group: Tab lands on the chosen one (or the first), the arrows choose
    the next, each said by its name (ADR 0039). */
function TileRow({
  label,
  tiles,
  value,
  onChoose,
  testID,
}: {
  label: string;
  tiles: readonly Tile[];
  value: string;
  onChoose: (value: string) => void;
  testID: string;
}) {
  const chosen = tiles.some((tile) => tile.value === value) ? value : undefined;
  const options: RadioOption<string>[] = tiles.map((tile) => ({
    value: tile.value,
    label: tile.label,
    labelContent: <Frame chosen={tile.value === chosen}>{tile.picture}</Frame>,
    testID: tile.testID,
  }));
  return (
    <RadioGroup<string>
      label={label}
      variant="chip"
      value={chosen}
      onValueChange={onChoose}
      options={options}
      optionStyle={TILE_RADIO}
      style={ROW}
      testID={testID}
    />
  );
}

/** The colour picker: hue, saturation and brightness, and the colour's hex, as a Mac's colour
    panel offers them; the keyboard reaches each, the screen reader says each with its value. */
function ColorPicker({
  open,
  initial,
  words,
  onClose,
  onChoose,
  testID,
}: {
  open: boolean;
  initial: string;
  words: Words;
  onClose: () => void;
  onChoose: (hex: string) => void;
  testID: string;
}) {
  const [hsv, setHsv] = React.useState(() => hexToHsv(initial));
  const [text, setText] = React.useState(`#${initial.toUpperCase()}`);
  React.useEffect(() => {
    if (open) {
      setHsv(hexToHsv(initial));
      setText(`#${initial.toUpperCase()}`);
    }
  }, [open, initial]);
  const hex = hsvToHex(hsv.h, hsv.s, hsv.v);
  const change = (next: Partial<typeof hsv>) => {
    const value = { ...hsv, ...next };
    setHsv(value);
    setText(`#${hsvToHex(value.h, value.s, value.v).toUpperCase()}`);
  };
  const percent = (value: number) => `${Math.round(value)} %`;
  const typed = normalizeHex(text);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={words.picker.title}
      label={words.picker.label}
      actions={[
        { label: words.picker.cancel, color: "cancel", testID: `${testID}-cancel` },
        { label: words.picker.done, color: "default", onPress: () => onChoose(hex), testID: `${testID}-done` },
      ]}
      testID={testID}
    >
      <View style={{ gap: 12 }}>
        <View
          style={{ height: 56, borderRadius: RADIUS, backgroundColor: `#${hex}` }}
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        />
        <Slider
          label={words.picker.hue}
          accessibilityLabel={words.picker.hue}
          value={Math.round(hsv.h)}
          min={0}
          max={359}
          keyboardStep={5}
          onValueChange={(h) => change({ h })}
          formatValue={(h) => `${Math.round(h)}°`}
          minimumTrackTintColor={`#${hsvToHex(hsv.h, 1, 1)}`}
          showTooltip={false}
          testID={`${testID}-hue`}
        />
        <Slider
          label={words.picker.saturation}
          accessibilityLabel={words.picker.saturation}
          value={Math.round(hsv.s * 100)}
          min={0}
          max={100}
          keyboardStep={5}
          onValueChange={(s) => change({ s: s / 100 })}
          formatValue={percent}
          minimumTrackTintColor={`#${hex}`}
          showTooltip={false}
          testID={`${testID}-saturation`}
        />
        <Slider
          label={words.picker.brightness}
          accessibilityLabel={words.picker.brightness}
          value={Math.round(hsv.v * 100)}
          min={0}
          max={100}
          keyboardStep={5}
          onValueChange={(v) => change({ v: v / 100 })}
          formatValue={percent}
          minimumTrackTintColor={`#${hex}`}
          showTooltip={false}
          testID={`${testID}-brightness`}
        />
        <TextField invalid={typed === null}>
          <TextFieldInput
            label={words.picker.hex}
            accessibilityHint={words.picker.hexHint}
            value={text}
            onChangeText={(value) => {
              setText(value);
              const parsed = normalizeHex(value);
              if (parsed) setHsv(hexToHsv(parsed));
            }}
            autoCapitalize="characters"
            autoCorrect={false}
            maxLength={7}
            testID={`${testID}-hex`}
          />
        </TextField>
      </View>
    </Dialog>
  );
}

/** A colour well: a button showing a colour, said with what it sets and the colour. */
function ColorWell({
  color,
  label,
  onPress,
  testID,
}: {
  color: string;
  label: string;
  onPress: () => void;
  testID: string;
}) {
  const theme = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${label}, #${color.toUpperCase()}`}
      onPress={onPress}
      focusable
      testID={testID}
      style={{
        width: 44,
        height: 24,
        borderRadius: 6,
        borderWidth: 1,
        borderColor: theme.colors.border,
        backgroundColor: `#${color}`,
      }}
    />
  );
}

/** A photo's tile picture; a folder's is its first image, with a folder's glyph. */
function AddedPicture({
  platform,
  item,
  size,
}: {
  platform: WallpaperActions | undefined;
  item: AddedWallpaper;
  size: { width: number; height: number };
}) {
  const images = useFolder(platform, item.kind === "folder" ? item.path : null);
  const uri = item.kind === "photo" ? fileUri(item.path) : (images[0] ?? null);
  return (
    <Picture platform={platform} uri={uri} size={size}>
      {item.kind === "folder" ? (
        <View
          style={{
            position: "absolute",
            right: 4,
            bottom: 4,
            width: 22,
            height: 22,
            borderRadius: 11,
            backgroundColor: "rgba(0,0,0,0.45)",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <RiFolder3Line width={13} fill="#ffffff" />
        </View>
      ) : null}
    </Picture>
  );
}

/** The current wallpaper's picture at the top, as it shows now. */
function CurrentPicture({
  platform,
  choice,
  catalog,
  dark,
}: {
  platform: WallpaperActions | undefined;
  choice: WallpaperChoice;
  catalog: WallpaperCatalog;
  dark: boolean;
}) {
  const folder = useFolder(platform, choice.kind === "folder" ? choice.path : null);
  switch (choice.kind) {
    case "color":
      return <Picture platform={platform} color={choice.color} size={PREVIEW} />;
    case "photo":
      return (
        <Picture platform={platform} uri={fileUri(choice.path)} fit={choice.fit} color={choice.margin} size={PREVIEW} />
      );
    case "folder":
      return <Picture platform={platform} uri={folder[0] ?? null} fit={choice.fit} color={choice.margin} size={PREVIEW} />;
    case "shuffle": {
      const category = catalog.categories.find((c) => c.id === choice.category);
      return (
        <Mosaic
          platform={platform}
          uris={category?.wallpapers.map((w) => wallpaperPicture(w, null, dark)) ?? []}
          colors={catalog.colors.map((c) => normalizeHex(c.color) ?? "000000")}
          size={PREVIEW}
        />
      );
    }
    case "system": {
      const wallpaper = findWallpaper(catalog, choice.id);
      return (
        <Picture platform={platform} uri={wallpaper ? wallpaperPicture(wallpaper, choice.variant, dark) : null} size={PREVIEW} />
      );
    }
    default:
      return <Picture platform={platform} size={PREVIEW} />;
  }
}

/** What the current choice is called, and what kind of wallpaper it is. */
function describe(
  choice: WallpaperChoice,
  catalog: WallpaperCatalog,
  words: Words,
): { name: string; kind: string } {
  switch (choice.kind) {
    case "color": {
      const named = catalog.colors.find((c) => normalizeHex(c.color) === choice.color);
      return { name: named?.name ?? words.custom(choice.color), kind: words.kinds.color };
    }
    case "photo":
      return { name: baseName(choice.path), kind: words.kinds.photo };
    case "folder":
      return { name: baseName(choice.path), kind: words.kinds.folder };
    case "shuffle": {
      const name =
        choice.category === COLORS_CATEGORY
          ? words.shuffleColors
          : words.shuffle(catalog.categories.find((c) => c.id === choice.category)?.name ?? choice.category);
      return { name, kind: words.kinds.shuffle };
    }
    case "system": {
      const wallpaper = findWallpaper(catalog, choice.id);
      const kind =
        wallpaper?.automatic === "sun"
          ? words.kinds.dynamic
          : wallpaper?.automatic === "appearance"
            ? words.kinds.appearance
            : words.kinds.still;
      return { name: wallpaper?.name ?? choice.id, kind };
    }
    default:
      return { name: "", kind: words.kinds.still };
  }
}

export function WallpaperPage({ snapshot, actions, strings }: Props) {
  const words = strings.wallpaper;
  const theme = useTheme();
  const dark = useColorScheme() === "dark";
  const catalog = snapshot.wallpapers;
  const platform = actions.wallpaper;
  const choice = shownChoice(snapshot.preferences.wallpaper, catalog);
  const value = tileValue(choice);
  const set = React.useCallback(
    (next: WallpaperChoice) => actions.setPreference("wallpaper", choiceText(next)),
    [actions],
  );
  const choose = (tile: string) => {
    const next = chooseTile(tile, choice);
    if (next) set(next);
  };
  // Categories shown whole (Show All), by id.
  const [whole, setWhole] = React.useState<ReadonlySet<string>>(() => new Set());
  const toggleWhole = (id: string) =>
    setWhole((shown) => {
      const next = new Set(shown);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  // The colour picker: what it picks (the wallpaper's colour, or the colour around a photo).
  const [picking, setPicking] = React.useState<"color" | "margin" | null>(null);
  const added = addedWithChosen(snapshot.addedWallpapers ?? [], choice);
  const add = async (kind: AddedWallpaper["kind"]) => {
    const pick = kind === "photo" ? platform?.pickPhoto : platform?.pickFolder;
    const path = pick ? await pick() : null;
    if (!path) return;
    platform?.setAdded?.(withAdded(snapshot.addedWallpapers ?? [], { kind, path }));
    const next = chooseTile(`${kind}:${path}`, choice);
    if (next) set(next);
  };
  const chosenAdded =
    choice.kind === "photo" || choice.kind === "folder"
      ? (snapshot.addedWallpapers ?? []).find((item) => item.kind === choice.kind && item.path === choice.path)
      : undefined;
  const { name, kind } = describe(choice, catalog, words);
  const system = choice.kind === "system" ? findWallpaper(catalog, choice.id) : undefined;
  const shuffleOf = (categoryId: string, pictures: readonly string[], colors: readonly string[], label: string): Tile => ({
    value: `shuffle:${categoryId}`,
    label,
    picture: <Mosaic platform={platform} uris={pictures} colors={colors} size={TILE} />,
    testID: `wallpaper-shuffle-${categoryId}`,
  });
  const colors = colorsWithChosen(catalog, choice);
  const catalogColors = catalog.colors.map((c) => normalizeHex(c.color) ?? "000000");
  const pickerInitial =
    picking === "margin" && (choice.kind === "photo" || choice.kind === "folder")
      ? choice.margin
      : choice.kind === "color"
        ? choice.color
        : (catalogColors[0] ?? "1f5fd6");

  return (
    <View style={PAGE}>
      <GroupedSection
        footer={
          system && system.variants.length > 1
            ? system.automatic === "sun"
              ? words.automaticSun
              : system.automatic === "appearance"
                ? words.automaticAppearance
                : undefined
            : undefined
        }
      >
        <GroupedSectionItem
          title={name}
          testID="wallpaper-current"
          content={
            <View
              style={{ flexDirection: "row", alignItems: "center", gap: 16 }}
              accessible
              accessibilityRole="image"
              accessibilityLabel={`${words.preview(name)}, ${kind}`}
              testID="wallpaper-current-preview"
            >
              <CurrentPicture platform={platform} choice={choice} catalog={catalog} dark={dark} />
              <View style={{ flex: 1, gap: 2 }}>
                <Text variant="title-3-semibold" numberOfLines={2} testID="wallpaper-current-name">
                  {name}
                </Text>
                <Text variant="body-regular" style={{ color: theme.colors.textSecondary }}>
                  {kind}
                </Text>
              </View>
            </View>
          }
        />
        {system && system.variants.length > 1 ? (
          <GroupedSectionItem
            title={words.variant}
            testID="wallpaper-variant-row"
            rightElement={
              <SettingsSelect
                label={words.variant}
                value={choice.kind === "system" ? (choice.variant ?? AUTOMATIC) : AUTOMATIC}
                items={[
                  { value: AUTOMATIC, label: words.automatic },
                  ...system.variants.map((variant) => ({ value: variant.id, label: variant.name })),
                ]}
                onChange={(variant) =>
                  set({ kind: "system", id: system.id, variant: variant === AUTOMATIC ? null : variant })
                }
                testID="wallpaper-variant"
              />
            }
          />
        ) : null}
        {choice.kind === "photo" || choice.kind === "folder" ? (
          <GroupedSectionItem
            title={words.fit}
            testID="wallpaper-fit-row"
            rightElement={
              <SettingsSelect<WallpaperFit>
                label={words.fit}
                value={choice.fit}
                items={FITS.map((fit) => ({ value: fit, label: words.fits[fit] }))}
                onChange={(fit) => set({ ...choice, fit })}
                testID="wallpaper-fit"
              />
            }
          />
        ) : null}
        {(choice.kind === "photo" || choice.kind === "folder") && choice.fit !== "fill" && choice.fit !== "stretch" ? (
          <GroupedSectionItem
            title={words.margin}
            testID="wallpaper-margin-row"
            rightElement={
              <ColorWell
                color={choice.margin}
                label={words.margin}
                onPress={() => setPicking("margin")}
                testID="wallpaper-margin"
              />
            }
          />
        ) : null}
        {choice.kind === "folder" || choice.kind === "shuffle" ? (
          <GroupedSectionItem
            title={words.interval}
            testID="wallpaper-interval-row"
            rightElement={
              <SettingsSelect
                label={words.interval}
                value={String(choice.every)}
                items={SHUFFLE_EVERY.map((every) => ({ value: String(every), label: words.every(every) }))}
                onChange={(text) => {
                  const every: ShuffleEvery = text === "login" ? "login" : Number(text);
                  set({ ...choice, every });
                }}
                testID="wallpaper-interval"
              />
            }
          />
        ) : null}
        {choice.kind === "color" ? (
          <GroupedSectionItem
            title={words.colors}
            testID="wallpaper-color-row"
            rightElement={
              <ColorWell
                color={choice.color}
                label={words.kinds.color}
                onPress={() => setPicking("color")}
                testID="wallpaper-color-well"
              />
            }
          />
        ) : null}
      </GroupedSection>

      <GroupedSection footer={words.allDesktopsDescription}>
        <GroupedSectionItem
          title={words.allDesktops}
          testID="wallpaper-all-desktops"
          rightElement={
            <Switch
              checked={snapshot.preferences.wallpaperAllDesktops}
              onCheckedChange={(on) => actions.setPreference("wallpaperAllDesktops", on)}
              accessibilityLabel={words.allDesktops}
              testID="wallpaper-all-desktops-switch"
            />
          }
        />
      </GroupedSection>

      {catalog.categories.length === 0 && catalog.colors.length === 0 ? (
        <GroupedSection>
          <GroupedSectionItem title={words.unavailable} titleNumberOfLines={0} testID="wallpaper-unavailable" />
        </GroupedSection>
      ) : null}

      {catalog.categories.map((category) => {
        const all = whole.has(category.id);
        const shown = shownTiles(category.wallpapers, all, (w) => value === `system:${w.id}`);
        const pictures = category.wallpapers.map((w) => wallpaperPicture(w, null, dark));
        const tiles: Tile[] = [
          ...shown.map((wallpaper) => ({
            value: `system:${wallpaper.id}`,
            label: wallpaper.name,
            picture: <Picture platform={platform} uri={wallpaperPicture(wallpaper, null, dark)} size={TILE} />,
            testID: `wallpaper-${wallpaper.id}`,
          })),
          shuffleOf(category.id, pictures, [], words.shuffle(category.name)),
        ];
        return (
          <View key={category.id}>
            <SectionHeading title={category.name}>
              {category.wallpapers.length > shown.length || all ? (
                <Button
                  size="xs"
                  appearance="plain"
                  tone="neutral"
                  onPress={() => toggleWhole(category.id)}
                  testID={`wallpaper-show-all-${category.id}`}
                >
                  {all ? words.showLess : words.showAll(category.wallpapers.length)}
                </Button>
              ) : null}
            </SectionHeading>
            <GroupedSection>
              <GroupedSectionItem
                title={category.name}
                testID={`wallpaper-category-${category.id}`}
                content={
                  <TileRow
                    label={words.row(category.name)}
                    tiles={tiles}
                    value={value}
                    onChoose={choose}
                    testID={`wallpaper-row-${category.id}`}
                  />
                }
              />
            </GroupedSection>
          </View>
        );
      })}

      {platform?.pickPhoto || platform?.pickFolder || added.length > 0 ? (
        <GroupedSection title={words.yourPhotos} footer={added.length === 0 ? words.noPhotos : undefined}>
          <GroupedSectionItem
            title={words.yourPhotos}
            testID="wallpaper-your-photos"
            content={
              <View style={{ gap: 8 }}>
                {added.length > 0 ? (
                  <TileRow
                    label={words.yourPhotos}
                    tiles={added.map((item) => ({
                      value: `${item.kind}:${item.path}`,
                      label: item.kind === "folder" ? words.folder(baseName(item.path)) : baseName(item.path),
                      picture: <AddedPicture platform={platform} item={item} size={TILE} />,
                      testID: `wallpaper-added-${item.kind}-${baseName(item.path)}`,
                    }))}
                    value={value}
                    onChoose={choose}
                    testID="wallpaper-row-added"
                  />
                ) : null}
                <View style={BUTTONS}>
                  {platform?.pickPhoto ? (
                    <Button
                      size="sm"
                      appearance="outline"
                      tone="neutral"
                      leadingIcon={RiImageAddLine}
                      onPress={() => void add("photo")}
                      testID="wallpaper-add-photo"
                    >
                      {words.addPhoto}
                    </Button>
                  ) : null}
                  {platform?.pickFolder ? (
                    <Button
                      size="sm"
                      appearance="outline"
                      tone="neutral"
                      leadingIcon={RiFolder3Line}
                      onPress={() => void add("folder")}
                      testID="wallpaper-add-folder"
                    >
                      {words.addFolder}
                    </Button>
                  ) : null}
                  {chosenAdded && platform?.setAdded ? (
                    <Button
                      size="sm"
                      appearance="plain"
                      tone="neutral"
                      leadingIcon={RiSubtractLine}
                      accessibilityLabel={words.removeNamed(baseName(chosenAdded.path))}
                      onPress={() =>
                        platform.setAdded?.(
                          (snapshot.addedWallpapers ?? []).filter(
                            (item) => !(item.kind === chosenAdded.kind && item.path === chosenAdded.path),
                          ),
                        )
                      }
                      testID="wallpaper-remove-added"
                    >
                      {words.remove}
                    </Button>
                  ) : null}
                </View>
              </View>
            }
          />
        </GroupedSection>
      ) : null}

      {colors.length > 0 ? (
        <GroupedSection title={words.colors}>
          <GroupedSectionItem
            title={words.colors}
            testID="wallpaper-colors"
            content={
              <View style={{ gap: 8 }}>
                <TileRow
                  label={words.colors}
                  tiles={[
                    ...colors.map((color) => ({
                      value: `color:${color}`,
                      label:
                        catalog.colors.find((c) => normalizeHex(c.color) === color)?.name ?? words.custom(color),
                      picture: <Picture platform={platform} color={color} size={TILE} />,
                      testID: `wallpaper-color-${color}`,
                    })),
                    shuffleOf(COLORS_CATEGORY, [], catalogColors, words.shuffleColors),
                  ]}
                  value={value}
                  onChoose={choose}
                  testID="wallpaper-row-colors"
                />
                <View style={BUTTONS}>
                  <Button
                    size="sm"
                    appearance="outline"
                    tone="neutral"
                    leadingIcon={RiPaletteLine}
                    onPress={() => setPicking("color")}
                    testID="wallpaper-custom-color"
                  >
                    {words.customColor}
                  </Button>
                </View>
              </View>
            }
          />
        </GroupedSection>
      ) : null}

      <ColorPicker
        open={picking !== null}
        initial={pickerInitial}
        words={words}
        onClose={() => setPicking(null)}
        onChoose={(hex) => {
          const target = picking;
          setPicking(null);
          if (target === "margin" && (choice.kind === "photo" || choice.kind === "folder")) {
            set({ ...choice, margin: hex });
          } else {
            set({ kind: "color", color: hex });
          }
        }}
        testID="wallpaper-color-picker"
      />
    </View>
  );
}
