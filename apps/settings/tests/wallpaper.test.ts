import { test } from "node:test";
import assert from "node:assert/strict";
import {
  addedText,
  addedWithChosen,
  chooseTile,
  choiceText,
  colorsWithChosen,
  fileUri,
  hexToHsv,
  hsvToHex,
  isDark,
  MAX_ADDED,
  normalizeHex,
  parseAdded,
  parseChoice,
  ROW_TILES,
  shownChoice,
  shownTiles,
  tileValue,
  uriPath,
  validPath,
  withAdded,
  type WallpaperCatalog,
  type WallpaperChoice,
} from "../src/wallpaper.ts";
import { CATALOGUES } from "../src/messages.ts";

const CATALOG: WallpaperCatalog = {
  default: "oxyos",
  categories: [
    {
      id: "dynamic",
      name: "Dynamic Wallpapers",
      wallpapers: [
        { id: "oxyos", name: "OxyOS", automatic: "appearance", preview: "file:///w/oxyos/light.svg", variants: [] },
        { id: "solar-gradients", name: "Solar Gradients", automatic: "sun", preview: "file:///w/solar-gradients/midday.jpg", variants: [] },
      ],
    },
  ],
  colors: [
    { color: "#1F4FD8", name: "Blue" },
    { color: "#000000", name: "Black" },
  ],
};

test("every choice the grammar has reads back to its own text", () => {
  for (const text of [
    "",
    "oxyos",
    "solar-gradients/night",
    "color:1f4fd8",
    "photo:fill:000000:/home/nate/Pictures/a.jpg",
    "photo:tile:ffffff:/home/nate/Pictures/with:colons and spaces.png",
    "folder:fit:202020:login:/home/nate/Pictures/Trips",
    "folder:center:202020:1440:/media/nate/USB/Photos",
    "shuffle:gradients:60",
    "shuffle:colors:login",
  ]) {
    const choice = parseChoice(text);
    assert.ok(choice, text);
    assert.equal(choiceText(choice), text);
  }
});

test("what the session would refuse is no choice", () => {
  for (const text of [
    "Oxyos",
    "1solar",
    "solar-gradients/",
    "solar-gradients/night/extra",
    "color:1F4FD8",
    "color:12345",
    "photo:cover:000000:/a.jpg",
    "photo:fill:000000:relative.jpg",
    "photo:fill:000000:/a/../b.jpg",
    "photo:fill:000000:/a/./b.jpg",
    "photo:fill:000000:/a/",
    "photo:fill:000000:/a\nb.jpg",
    "photo:fill:/a.jpg",
    "folder:fill:000000:0:/a",
    "folder:fill:000000:10081:/a",
    "folder:fill:000000:05:/a",
    "shuffle:gradients",
    "shuffle:gradients:60:x",
    "wallpaper:x",
    `photo:fill:000000:/${"a".repeat(1024)}`,
  ]) {
    assert.equal(parseChoice(text), null, text);
  }
});

test("a path is at most 1024 bytes, counted in UTF-8", () => {
  assert.ok(validPath(`/${"a".repeat(1023)}`));
  assert.ok(!validPath(`/${"a".repeat(1024)}`));
  assert.ok(!validPath(`/${"ñ".repeat(512)}`));
  assert.ok(!validPath("/"));
});

test("the default shows as the catalog's default wallpaper; something unreadable too", () => {
  assert.deepEqual(shownChoice("", CATALOG), { kind: "system", id: "oxyos", variant: null });
  assert.deepEqual(shownChoice("photo:nope", CATALOG), { kind: "system", id: "oxyos", variant: null });
  assert.deepEqual(shownChoice("solar-gradients/dawn", CATALOG), { kind: "system", id: "solar-gradients", variant: "dawn" });
});

test("a tile stays chosen while its options change, and choosing one keeps what still applies", () => {
  const photo = parseChoice("photo:center:ffffff:/p/a.jpg")!;
  assert.equal(tileValue(photo), tileValue(parseChoice("photo:fill:000000:/p/a.jpg")!));
  assert.equal(tileValue(parseChoice("solar-gradients/night")!), tileValue(parseChoice("solar-gradients")!));
  // Another photo keeps the fit and the colour behind it.
  assert.equal(choiceText(chooseTile("photo:/p/b.jpg", photo)!), "photo:center:ffffff:/p/b.jpg");
  // A folder after a shuffle keeps its interval.
  const shuffle = parseChoice("shuffle:gradients:15")!;
  assert.equal(choiceText(chooseTile("folder:/p/Trips", shuffle)!), "folder:fill:000000:15:/p/Trips");
  // A system wallpaper is chosen Automatic; a shuffle starts hourly.
  assert.equal(choiceText(chooseTile("system:solar-gradients", photo)!), "solar-gradients");
  assert.equal(choiceText(chooseTile("shuffle:dynamic", photo)!), "shuffle:dynamic:60");
  assert.equal(chooseTile("photo:relative", photo), null);
});

test("colours: the catalog's, and the person's own after them", () => {
  assert.deepEqual(colorsWithChosen(CATALOG, { kind: "default" }), ["1f4fd8", "000000"]);
  assert.deepEqual(colorsWithChosen(CATALOG, { kind: "color", color: "000000" }), ["1f4fd8", "000000"]);
  assert.deepEqual(colorsWithChosen(CATALOG, { kind: "color", color: "abcdef" }), ["1f4fd8", "000000", "abcdef"]);
  assert.equal(normalizeHex("#ABC"), "aabbcc");
  assert.equal(normalizeHex(" 1F4FD8 "), "1f4fd8");
  assert.equal(normalizeHex("#12345"), null);
});

test("hue, saturation and brightness round-trip", () => {
  for (const hex of ["ff0000", "00ff00", "0000ff", "1f4fd8", "808080", "000000", "ffffff", "c86e2a"]) {
    const { h, s, v } = hexToHsv(hex);
    assert.equal(hsvToHex(h, s, v), hex);
  }
  assert.deepEqual(hexToHsv("ff0000"), { h: 0, s: 1, v: 1 });
  assert.ok(isDark("1f4fd8"));
  assert.ok(!isDark("f5f5f5"));
});

test("file URIs and paths", () => {
  assert.equal(fileUri("/home/nate/My Photos/a#1.jpg"), "file:///home/nate/My%20Photos/a%231.jpg");
  assert.equal(uriPath("file:///home/nate/My%20Photos/a%231.jpg"), "/home/nate/My Photos/a#1.jpg");
  assert.equal(uriPath("file:///home/nate/Trips/"), "/home/nate/Trips");
  assert.equal(uriPath("https://example.com/a.jpg"), null);
});

test("the added photos and folders: the latest first, never twice, at most MAX_ADDED, kept as JSON", () => {
  let list = withAdded([], { kind: "photo", path: "/p/a.jpg" });
  list = withAdded(list, { kind: "folder", path: "/p" });
  list = withAdded(list, { kind: "photo", path: "/p/a.jpg" });
  assert.deepEqual(list, [
    { kind: "photo", path: "/p/a.jpg" },
    { kind: "folder", path: "/p" },
  ]);
  for (let i = 0; i < 40; i++) list = withAdded(list, { kind: "photo", path: `/p/${i}.jpg` });
  assert.equal(list.length, MAX_ADDED);
  assert.deepEqual(parseAdded(addedText(list)), list);
  assert.deepEqual(parseAdded('{"added":[{"kind":"photo","path":"rel"},{"kind":"x","path":"/a"},{"kind":"folder","path":"/a"}]}'), [
    { kind: "folder", path: "/a" },
  ]);
  assert.deepEqual(parseAdded("not json"), []);
  const chosen: WallpaperChoice = { kind: "photo", fit: "fill", margin: "000000", path: "/elsewhere.jpg" };
  assert.deepEqual(addedWithChosen([{ kind: "folder", path: "/a" }], chosen), [
    { kind: "photo", path: "/elsewhere.jpg" },
    { kind: "folder", path: "/a" },
  ]);
});

test("a row shows its first tiles, the chosen one among them, or all", () => {
  const items = Array.from({ length: 9 }, (_, i) => i);
  assert.deepEqual(shownTiles(items, false, (i) => i === 2), [0, 1, 2, 3]);
  assert.deepEqual(shownTiles(items, false, (i) => i === 7), [0, 1, 2, 7]);
  assert.deepEqual(shownTiles(items, true, () => false), items);
  assert.equal(shownTiles(items, false, () => false).length, ROW_TILES);
});

test("every wallpaper word is in both languages", () => {
  const keys = (value: unknown): string[] =>
    value && typeof value === "object"
      ? Object.entries(value).flatMap(([k, v]) => [k, ...keys(v).map((s) => `${k}.${s}`)])
      : [];
  assert.deepEqual(keys(CATALOGUES.es.wallpaper).sort(), keys(CATALOGUES.en.wallpaper).sort());
  assert.equal(CATALOGUES.es.wallpaper.showAll(12), "Mostrar todo (12)");
  assert.equal(CATALOGUES.en.wallpaper.every(1440), "Every day");
  assert.equal(CATALOGUES.es.wallpaper.every(15), "Cada 15 minutos");
});
