import { test } from "node:test";
import assert from "node:assert/strict";
import { shellMessages } from "../src/messages.ts";
import { chordLabel } from "../src/chord.ts";
import {
  windowMenuRows,
  appMenuCommand,
  applicationMenus,
  applicationName,
  applicationsShown,
  desktopMenus,
  type ToldMenuBar,
  type ToldMenuEntry,
} from "../src/app-menu.ts";
import type { ShellMenu, ShellMenuItem } from "../src/model.ts";

const en = shellMessages("en-US");
const es = shellMessages("es-ES");

/** A menu's rows as the person reads them: label and chord, "(off)" when disabled, "—". */
function read(items: readonly ShellMenuItem[]): string[] {
  return items.map((item) => {
    if (item.kind === "separator") return "—";
    const chord = item.kind !== "submenu" && item.shortcut ? ` ${item.shortcut}` : "";
    const off = item.enabled === false ? " (off)" : "";
    return `${item.label}${chord}${off}`;
  });
}

const titles = (menus: readonly ShellMenu[]) => menus.map((menu) => menu.label);

const entry = (parent: number, kind: ToldMenuEntry["kind"], label = "", more: Partial<ToldMenuEntry> = {}): ToldMenuEntry => ({
  parent,
  kind,
  enabled: true,
  checkable: false,
  checked: false,
  label,
  shortcut: "",
  ...more,
});

const BOTH = { others: true, hidden: true };

test("one chord notation: key names in a Mac's order, in the session's language", () => {
  assert.equal(chordLabel("Mod+Shift+Z", en), "Ctrl+Shift+Z");
  assert.equal(chordLabel("Mod+Shift+Z", es), "Ctrl+Mayús+Z");
  // Bloom's canonical order and the system's: Ctrl, Alt, Shift, Super, whatever the order given.
  assert.equal(chordLabel("Mod+Alt+Shift+V", es), "Ctrl+Alt+Mayús+V");
  assert.equal(chordLabel(["Shift", "Alt", "Super", "Q"], en), "Alt+Shift+Super+Q");
  assert.equal(chordLabel(["Super", "Ctrl", "Q"], en), "Ctrl+Super+Q");
  assert.equal(chordLabel("Meta+H", en), "Super+H");
  // Keys: a letter in capitals, punctuation as it is, named keys in the language, arrows.
  assert.equal(chordLabel("Mod+,", es), "Ctrl+,");
  assert.equal(chordLabel("Mod+n", en), "Ctrl+N");
  assert.equal(chordLabel("Mod+Backspace", en), "Ctrl+Backspace");
  assert.equal(chordLabel("Mod+Backspace", es), "Ctrl+Retroceso");
  assert.equal(chordLabel("Mod+Delete", es), "Ctrl+Supr");
  assert.equal(chordLabel("Mod+Enter", es), "Ctrl+Intro");
  assert.equal(chordLabel("Mod+Space", es), "Ctrl+Espacio");
  assert.equal(chordLabel("Alt+ArrowUp", en), "Alt+↑");
  assert.equal(chordLabel("Mod+Plus", en), "Ctrl++");
  assert.equal(chordLabel("Mod++", en), "Ctrl++");
  assert.equal(chordLabel("F5", en), "F5");
  assert.equal(chordLabel("", en), "");
});

test("an application that declares no menus has its own: About, Settings…, Hide, Hide Others, Show All, Quit", () => {
  const bar: ToldMenuBar = { window: 4, serial: 0, items: [] };
  const menus = applicationMenus(bar, "Settings", en, BOTH);
  assert.deepEqual(titles(menus), ["Settings", "Window"]);
  assert.deepEqual(read(menus[0].items), [
    "About Settings",
    "—",
    "Settings… Ctrl+, (off)",
    "—",
    "Hide Settings Super+H",
    "Hide Others Alt+Super+H",
    "Show All",
    "—",
    "Quit Settings Super+Q",
  ]);
  // Without its own About, the system's standard panel; Settings… is not offered.
  assert.deepEqual(appMenuCommand(menus[0].items[0].id), { kind: "about", window: 4 });
  assert.equal(appMenuCommand(menus[0].items[2].id), null);
  assert.deepEqual(appMenuCommand(menus[0].items[4].id), { kind: "hide", window: 4 });
  assert.deepEqual(appMenuCommand(menus[0].items[5].id), { kind: "hideOthers", window: 4 });
  assert.deepEqual(appMenuCommand(menus[0].items[6].id), { kind: "showAll" });
  assert.deepEqual(appMenuCommand(menus[0].items[8].id), { kind: "quit", window: 4 });
  // In Spanish.
  assert.deepEqual(read(applicationMenus(bar, "Ajustes", es, BOTH)[0].items), [
    "Acerca de Ajustes",
    "—",
    "Ajustes… Ctrl+, (off)",
    "—",
    "Ocultar Ajustes Super+H",
    "Ocultar otros Alt+Super+H",
    "Mostrar todo",
    "—",
    "Salir de Ajustes Super+Q",
  ]);
});

test("Hide Others with no other application shown, and Show All with none hidden, are not offered", () => {
  const bar: ToldMenuBar = { window: 4, serial: 0, items: [] };
  const rows = read(applicationMenus(bar, "Allo", en, { others: false, hidden: false })[0].items);
  assert.ok(rows.includes("Hide Others Alt+Super+H (off)"));
  assert.ok(rows.includes("Show All (off)"));
  const windows = [
    { id: "1", appId: "allo", title: "Allo", workspaceId: "0" },
    { id: "2", appId: "inbox", title: "Inbox", workspaceId: "0", minimized: true },
  ];
  assert.deepEqual(applicationsShown(windows, "allo"), { others: false, hidden: false });
  assert.deepEqual(applicationsShown([...windows, { id: "3", appId: "mention", title: "M", workspaceId: "0", hidden: true }], "allo"), {
    others: false,
    hidden: true,
  });
  assert.deepEqual(applicationsShown(windows, undefined), { others: true, hidden: false });
});

test("the application's own About and Settings… are hoisted into its menu, chosen in it", () => {
  // File: New, —, About Notes, Settings…; Edit: Undo; Notes: About, Settings (only hoisted).
  const items: ToldMenuEntry[] = [
    entry(-1, "menu", "File"),
    entry(0, "item", "New", { shortcut: "Mod+N" }),
    entry(0, "separator"),
    entry(0, "item", "About Notes", { role: "about" }),
    entry(0, "item", "Preferences…", { role: "settings", shortcut: "Mod+," }),
    entry(-1, "menu", "Edit"),
    entry(5, "item", "Undo", { shortcut: "Mod+Z", enabled: false }),
    entry(-1, "menu", "Empty"),
  ];
  const bar: ToldMenuBar = { window: 7, serial: 3, items };
  const menus = applicationMenus(bar, "Notes", es, BOTH);
  // A menu the application declared empty stays; File keeps New, its separator gone with them.
  assert.deepEqual(titles(menus), ["Notes", "File", "Edit", "Empty", "Ventana"]);
  assert.deepEqual(read(menus[0].items), [
    "Acerca de Notes",
    "—",
    "Ajustes… Ctrl+,",
    "—",
    "Ocultar Notes Super+H",
    "Ocultar otros Alt+Super+H",
    "Mostrar todo",
    "—",
    "Salir de Notes Super+Q",
  ]);
  assert.deepEqual(read(menus[1].items), ["New Ctrl+N"]);
  assert.deepEqual(read(menus[2].items), ["Undo Ctrl+Z (off)"]);
  // Chosen, each is the application's command: it runs there.
  assert.deepEqual(appMenuCommand(menus[0].items[0].id), { kind: "declared", window: 7, serial: 3, index: 3 });
  assert.deepEqual(appMenuCommand(menus[0].items[2].id), { kind: "declared", window: 7, serial: 3, index: 4 });
  // A menu that held only what was hoisted goes.
  const only: ToldMenuBar = {
    window: 7,
    serial: 4,
    items: [entry(-1, "menu", "Notes"), entry(0, "item", "Settings…", { role: "settings", enabled: false })],
  };
  const hoisted = applicationMenus(only, "Notes", en, BOTH);
  assert.deepEqual(titles(hoisted), ["Notes", "Window"]);
  // Its own state: a disabled Settings… stays disabled, with no chord it did not declare.
  assert.equal(read(hoisted[0].items)[2], "Settings… (off)");
});

test("declared menus keep their order, checks, submenus and chords, in the bar's notation", () => {
  const items: ToldMenuEntry[] = [
    entry(-1, "menu", "Archivo"),
    entry(0, "item", "Nuevo", { shortcut: "Mod+N" }),
    entry(0, "submenu", "Recientes"),
    entry(2, "item", "notas.txt"),
    entry(-1, "menu", "Edición"),
    entry(4, "item", "Ajustar líneas", { checkable: true, checked: true }),
    entry(4, "item", "Pegar sin formato", { shortcut: "Mod+Alt+Shift+V" }),
  ];
  const menus = applicationMenus({ window: 2, serial: 9, items }, "Allo", es, BOTH);
  assert.deepEqual(titles(menus), ["Allo", "Archivo", "Edición", "Ventana"]);
  assert.deepEqual(read(menus[1].items), ["Nuevo Ctrl+N", "Recientes"]);
  const recent = menus[1].items[1];
  assert.ok(recent.kind === "submenu");
  assert.deepEqual(read(recent.items), ["notas.txt"]);
  assert.deepEqual(read(menus[2].items), ["Ajustar líneas", "Pegar sin formato Ctrl+Alt+Mayús+V"]);
  const wrap = menus[2].items[0];
  assert.ok(wrap.kind !== "separator" && wrap.kind !== "submenu" && wrap.checked === true);
  assert.equal(appMenuCommand("window-menu:2:9:5")?.kind, "declared");
});

test("with no window active, the desktop's menu (Finder's role) and View", () => {
  const menus = desktopMenus(en, { trashItems: 2, shown: { others: true, hidden: false } });
  assert.deepEqual(titles(menus), ["Desktop", "View", "Window"]);
  assert.deepEqual(read(menus[0].items), ["Empty Trash…", "—", "Hide Others Alt+Super+H", "Show All (off)"]);
  assert.deepEqual(read(menus[1].items), ["Show Desktops", "Show Applications"]);
  assert.deepEqual(appMenuCommand(menus[0].items[0].id), { kind: "emptyTrash" });
  // Hide Others there hides every application.
  assert.deepEqual(appMenuCommand(menus[0].items[2].id), { kind: "hideOthers", window: 0 });
  // An empty Trash is not offered; a platform with none shows no row for it.
  assert.deepEqual(read(desktopMenus(es, { trashItems: 0, shown: BOTH })[0].items), [
    "Vaciar Papelera… (off)",
    "—",
    "Ocultar otros Alt+Super+H",
    "Mostrar todo",
  ]);
  assert.deepEqual(read(desktopMenus(es, { shown: BOTH })[0].items), ["Ocultar otros Alt+Super+H", "Mostrar todo"]);
  assert.deepEqual(titles(desktopMenus(es, { shown: BOTH })), ["Escritorio", "Ver", "Ventana"]);
});

test("an application's name: the registry's for one the session started, else its window's", () => {
  const apps = [{ id: "so.oxy.allo", name: "Allo" }];
  assert.equal(applicationName({ appId: "so.oxy.allo", title: "Note", launched: true }, apps), "Allo");
  assert.equal(applicationName({ appId: "so.oxy.allo", title: "Note", launched: false }, apps), "Note");
  assert.equal(applicationName({ appId: "x", title: "", launched: true }, apps), "x");
  assert.equal(applicationName(undefined, apps), "");
  assert.equal(appMenuCommand("view.overview"), null);
});

test("an application nothing names still has its own menu, under a word of the catalogue", () => {
  // A window whose process the session did not start, with no title and no id it claims.
  const name = applicationName({ appId: "", title: "", launched: false }, []);
  assert.equal(name, "");
  const bar: ToldMenuBar = { window: 3, serial: 0, items: [] };
  const menus = applicationMenus(bar, name, es, BOTH);
  assert.deepEqual(titles(menus), ["Aplicación", "Ventana"]);
  assert.deepEqual(read(menus[0].items).slice(0, 1), ["Acerca de Aplicación"]);
  assert.equal(read(menus[0].items).at(-1), "Salir de Aplicación Super+Q");
  assert.deepEqual(titles(applicationMenus(bar, "", en, BOTH)), ["Application", "Window"]);
});

/** A menu's rows as read(), and each submenu's rows after it, indented. */
function readDeep(items: readonly ShellMenuItem[]): string[] {
  return items.flatMap((item, at) => [
    read([item])[0],
    ...(item.kind === "submenu" ? readDeep(item.items).map((row) => `  ${row}`) : []),
  ]);
}

test("every app has the system's Window menu last: the compositor's commands with its own chords", () => {
  const bar: ToldMenuBar = { window: 5, serial: 0, items: [] };
  const desktops = { desktops: [{ id: "1", name: "Escritorio 1" }, { id: "2", name: "Escritorio 2" }], current: "1" };
  const menus = applicationMenus(bar, "Allo", es, BOTH, desktops);
  assert.equal(menus.at(-1)?.label, "Ventana");
  assert.deepEqual(readDeep(menus.at(-1)?.items ?? []), [
    "Minimizar Super+M",
    "Zoom",
    "—",
    "Rellenar Alt+Super+F",
    "Centrar Alt+Super+C",
    "Mover y redimensionar",
    "  Mitad izquierda Alt+Super+←",
    "  Mitad derecha Alt+Super+→",
    "  Mitad superior Alt+Super+↑",
    "  Mitad inferior Alt+Super+↓",
    "  —",
    "  Cuarto superior izquierdo",
    "  Cuarto superior derecho",
    "  Cuarto inferior izquierdo",
    "  Cuarto inferior derecho",
    "  —",
    "  Izquierda y derecha Alt+Mayús+Super+←",
    "  Derecha e izquierda Alt+Mayús+Super+→",
    "  Arriba y abajo Alt+Mayús+Super+↑",
    "  Abajo y arriba Alt+Mayús+Super+↓",
    "  Cuartos",
    "  —",
    "  Volver al tamaño anterior Alt+Super+R",
    "—",
    // The other desktops only: the window is on Escritorio 1.
    "Mover a Escritorio 2",
  ]);
  // English, one desktop: no Move to.
  const rows = readDeep(windowMenuRows(5, en, { desktops: [{ id: "1", name: "Desktop 1" }], current: "1" }));
  assert.deepEqual(rows.slice(0, 5), ["Minimize Super+M", "Zoom", "—", "Fill Alt+Super+F", "Center Alt+Super+C"]);
  assert.equal(rows.at(-1), "  Return to Previous Size Alt+Super+R");
  assert.ok(!rows.some((row) => row.startsWith("Move to")));
});

test("each Window menu row asks the compositor for the window, by its number", () => {
  const rows = windowMenuRows(5, en, { desktops: [{ id: "1", name: "Desktop 1" }, { id: "3", name: "Desktop 3" }], current: "1" });
  const ids = rows.flatMap((row) => (row.kind === "submenu" ? row.items : [row])).filter((row) => row.kind !== "separator");
  const commands = ids.map((row) => appMenuCommand(row.id));
  assert.deepEqual(commands[0], { kind: "minimize", window: 5 });
  assert.deepEqual(commands[1], { kind: "zoom", window: 5 });
  assert.deepEqual(commands[2], { kind: "layout", window: 5, layout: "fill" });
  assert.deepEqual(commands[3], { kind: "layout", window: 5, layout: "center" });
  assert.deepEqual(
    commands.filter((command) => command?.kind === "layout").map((command) => command?.kind === "layout" && command.layout),
    [
      "fill",
      "center",
      "left",
      "right",
      "top",
      "bottom",
      "topLeft",
      "topRight",
      "bottomLeft",
      "bottomRight",
      "leftAndRight",
      "rightAndLeft",
      "topAndBottom",
      "bottomAndTop",
      "quarters",
      "returnToPreviousSize",
    ],
  );
  assert.deepEqual(commands.at(-1), { kind: "moveToDesktop", window: 5, desktop: "3" });
  // A layout no menu offers is no command.
  assert.equal(appMenuCommand("window-command:layout:sideways:5"), null);
});

test("with no window active the Window menu is there and offers nothing", () => {
  const menus = desktopMenus(en, { shown: BOTH });
  const window = menus.at(-1);
  assert.equal(window?.label, "Window");
  const rows = (window?.items ?? []).flatMap((row) => (row.kind === "submenu" ? [row, ...row.items] : [row]));
  for (const row of rows) if (row.kind !== "separator") assert.equal(row.enabled, false, row.label);
  assert.deepEqual(read(window?.items ?? []).slice(0, 2), ["Minimize Super+M (off)", "Zoom (off)"]);
});

test("an app's own Window menu takes the system's rows first, as AppKit merges them", () => {
  const items: ToldMenuEntry[] = [
    entry(-1, "menu", "Window"),
    entry(0, "item", "Show Inspector", { shortcut: "Mod+I" }),
  ];
  const menus = applicationMenus({ window: 2, serial: 1, items }, "Notes", en, BOTH);
  assert.deepEqual(titles(menus), ["Notes", "Window"]);
  const rows = read(menus[1].items);
  assert.equal(rows[0], "Minimize Super+M");
  assert.deepEqual(rows.slice(-2), ["—", "Show Inspector Ctrl+I"]);
  assert.deepEqual(appMenuCommand(menus[1].items.at(-1)?.id ?? ""), { kind: "declared", window: 2, serial: 1, index: 1 });
});
