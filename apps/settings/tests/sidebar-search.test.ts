import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { declaredPage, finds, fold, searchedGroups, type DeclaredPage } from "../src/sidebar-search.ts";

const declared: DeclaredPage[] = JSON.parse(readFileSync(new URL("../searchable.json", import.meta.url), "utf8"));

const groups = [
  {
    key: "account",
    label: "Oxy Account",
    items: [{ key: "oxy-account", label: "Ada Lovelace", page: "oxy-account" }],
  },
  {
    key: "connections",
    label: "Connections",
    items: [
      { key: "wifi", label: "Wi-Fi", page: "wifi" },
      { key: "bluetooth", label: "Bluetooth", page: "bluetooth" },
    ],
  },
  {
    key: "personal",
    label: "Personal",
    items: [
      { key: "appearance", label: "Appearance", page: "appearance" },
      { key: "wallpaper", label: "Wallpaper", page: "wallpaper" },
      { key: "lock-screen", label: "Lock Screen", page: "lock-screen" },
    ],
  },
];

const shown = (query: string, language = "en") =>
  searchedGroups(groups, query, declared, language).flatMap((group) => group.items.map((item) => item.label));

test("text is compared without accents or case", () => {
  assert.equal(fold("Contraseña ÁRBOL"), "contrasena arbol");
});

test("a declared item opens the page its URL names", () => {
  assert.equal(declaredPage({ id: "settings:wifi", url: "oxyos-settings://wifi", titles: {}, keywords: {} }), "wifi");
  assert.equal(declaredPage({ id: "x", url: "https://oxy.so", titles: {}, keywords: {} }), null);
});

test("every word typed begins a word of what the page is known by", () => {
  assert.equal(finds("lock scr", ["Lock Screen"]), true);
  assert.equal(finds("scr lock", ["Lock Screen"]), true);
  assert.equal(finds("ock", ["Lock Screen"]), false);
  assert.equal(finds("fi", ["Wi-Fi"]), true);
  assert.equal(finds("wi-fi", ["Wi-Fi"]), true);
  assert.equal(finds("   ", ["anything"]), true);
});

test("a blank search leaves the sidebar as it is", () => {
  assert.equal(searchedGroups(groups, "  ", declared, "en"), groups);
});

test("the sidebar's names, the declared titles and keywords find pages, in their order", () => {
  assert.deepEqual(shown("appear"), ["Appearance"]);
  // A declared keyword: "dark" is Appearance's.
  assert.deepEqual(shown("dark"), ["Appearance"]);
  // The name the sidebar shows (the person's own) and the declared title both find the account.
  assert.deepEqual(shown("ada"), ["Ada Lovelace"]);
  assert.deepEqual(shown("oxy acc"), ["Ada Lovelace"]);
  assert.deepEqual(shown("zzz"), []);
  // Every group is kept, emptied, in order.
  assert.deepEqual(
    searchedGroups(groups, "wall", declared, "en").map((group) => [group.key, group.items.length]),
    [
      ["account", 0],
      ["connections", 0],
      ["personal", 1],
    ],
  );
});

test("in Spanish, the Spanish keywords, accents aside", () => {
  assert.deepEqual(shown("pantalla bloq", "es").includes("Lock Screen"), true);
  assert.deepEqual(shown("fondo", "es"), ["Wallpaper"]);
});
