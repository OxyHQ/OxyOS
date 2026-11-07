import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Image,
  Text,
  View,
  type AccessibilityActionEvent,
  type LayoutChangeEvent,
  type PointerEvent,
  type TextLayoutEvent,
} from "react-native";
import { Directory, File, Paths } from "expo-file-system";
import { DropZone, type DropTargetDragEvent, type DropTargetDropEvent } from "@oxy.so/bloom/drop-zone";
import { usePlatformDrag, usePlatformFiles, type PlatformFiles } from "@oxy.so/bloom/platform-surface";
import { BloomColorScope, useTheme } from "@oxy.so/bloom/theme";
import { RiFileImageLine } from "@oxy.so/bloom/icons/RiFileImageLine";
import { RiFileLine } from "@oxy.so/bloom/icons/RiFileLine";
import { RiFileMusicLine } from "@oxy.so/bloom/icons/RiFileMusicLine";
import { RiFilePdf2Line } from "@oxy.so/bloom/icons/RiFilePdf2Line";
import { RiFileTextLine } from "@oxy.so/bloom/icons/RiFileTextLine";
import { RiFileZipLine } from "@oxy.so/bloom/icons/RiFileZipLine";
import { RiFilmLine } from "@oxy.so/bloom/icons/RiFilmLine";
import { DRAG_SLOP } from "../app-drag";
import {
  arrange,
  arrowed,
  banded,
  cellOrigin,
  copyName,
  DESKTOP_CELL,
  DESKTOP_ICON,
  DESKTOP_NAME_LINE,
  DESKTOP_NAME_WIDTH,
  DESKTOP_PICTURE_BOX,
  droppedAt,
  folderDrop,
  gridOf,
  inBand,
  inGridOrder,
  isSecondPress,
  itemAt,
  kindOf,
  movedTo,
  NO_PRESS,
  openingOf,
  parentOf,
  pressed,
  pressTime,
  readArrangement,
  remembered,
  sameArrangement,
  sameUri,
  writeArrangement,
  type Arrangement,
  type Cell,
  type DesktopItem,
  type DesktopKind,
  type Direction,
  type LastPress,
  type Point,
} from "../desktop";
import type { ShellActions, ShellApplication } from "../model";
import type { ShellKeyboard } from "../platform";
import { useShellMessages } from "../useShellMessages";
import { useAccent } from "./accent";
import { FOLDER_ICON } from "./folder-icon";
import { DocumentIcon } from "./DocumentIcon";

/** What the top bar's desktop menus ask of the desktop (File › Open, File › Move to Trash,
    Edit › Select All), a new `id` for each. */
export type DesktopCommand = { id: number; action: "open" | "trash" | "select-all" };
/** What the top bar shows of the desktop: whether it has the keyboard (its menus are then the
    bar's), and whether anything is selected and anything is on it. */
export type DesktopState = { keyboard: boolean; selection: boolean; items: boolean };

type DesktopProps = {
  applications: readonly ShellApplication[];
  actions: ShellActions;
  keyboard?: ShellKeyboard;
  focusDesktop?: () => void;
  useDesktopKeyboard?: () => boolean;
  announce?: (text: string, polite: boolean) => void;
  command?: DesktopCommand;
  onState?: (state: DesktopState) => void;
};

/**
 * The desktop's files (docs/adr/0047-finder-drags-and-the-desktop.mdx), as a Mac's desktop shows
 * `~/Desktop`: its items as icons over the wallpaper, beneath every window, arranged as the person
 * left them, chosen, opened, dragged out and dropped on. It is the person's `desktop` place, read
 * through the platform's file service; where the platform has none (the browser preview), or the
 * person has no desktop folder, the desktop is the wallpaper alone.
 */
export const Desktop = memo(function Desktop(props: DesktopProps) {
  const files = usePlatformFiles();
  return files ? <DesktopPlace files={files} {...props} /> : null;
});

/** The person's desktop folder, followed as the places change. */
function DesktopPlace({ files, ...props }: DesktopProps & { files: PlatformFiles }) {
  const [places, setPlaces] = useState<{ desktop: string | null; all: string[] }>({ desktop: null, all: [] });
  useEffect(() => {
    let live = true;
    const read = () =>
      files.places().then(
        (found) => {
          if (!live) return;
          const desktop = found.find((place) => place.kind === "desktop")?.uri ?? null;
          const all = found.map((place) => place.uri);
          setPlaces((now) =>
            now.desktop === desktop && now.all.join("\n") === all.join("\n") ? now : { desktop, all },
          );
        },
        (error: unknown) => console.warn("the desktop could not read the places", { error }),
      );
    read();
    const stop = files.watchPlaces(read);
    return () => {
      live = false;
      stop();
    };
  }, [files]);
  if (!places.desktop) return null;
  return <DesktopFolder key={places.desktop} uri={places.desktop} places={places.all} {...props} />;
}

const DESKTOP_JSON = "desktop.json";

/** The folder's items as the desktop shows them: those whose names begin with a dot are not. */
function listed(uri: string): DesktopItem[] {
  try {
    return new Directory(uri)
      .list()
      .filter((entry) => !entry.name.startsWith("."))
      .map((entry) => ({
        name: entry.name,
        uri: entry.uri,
        folder: entry instanceof Directory,
        type: entry instanceof File ? entry.type : "",
      }))
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
  } catch (error) {
    console.warn("the desktop could not list its folder", { uri, error });
    return [];
  }
}

/** Where the shell keeps the arrangement: its own folder (`Paths.document`, its package's). */
function arrangementFile(): File | null {
  try {
    return new File(Paths.document, DESKTOP_JSON);
  } catch (error) {
    console.warn("the desktop has no folder of its own to keep its arrangement in", { error });
    return null;
  }
}

function readKept(): Arrangement {
  const file = arrangementFile();
  try {
    return file?.exists ? readArrangement(file.textSync()) : {};
  } catch (error) {
    console.warn("the desktop could not read its arrangement", { error });
    return {};
  }
}

function writeKept(kept: Arrangement) {
  const file = arrangementFile();
  if (!file) return;
  file.write(writeArrangement(kept)).catch((error: unknown) =>
    console.warn("the desktop could not keep its arrangement", { error }),
  );
}

/** The last name of a `file://` URI, decoded. */
function nameOf(uri: string): string {
  const name = uri.replace(/\/+$/, "").split("/").pop() ?? "";
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

type ViewRef = React.ComponentRef<typeof View>;

/** The desktop's latest drag of its icons (decision 6): which, the one gripped and where in its
    cell, and whether it is still going on. */
type OwnDrag = { names: string[]; gripped: string; grip: Point; live: boolean };

const FILES = ["files"] as const;
const DROP_ACTIONS = ["move", "copy"] as const;
const DRAG_ACTIONS = ["copy", "move", "link"] as const;
const noKeys = () => false;
const ARROWS: Readonly<Record<string, Direction>> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

/** A press on the desktop, while it may still become a drag or a band. */
type Press =
  | { kind: "icon"; name: string; at: Point; grip: Point; wasSelected: boolean; adds: boolean }
  | { kind: "band"; at: Point; base: string[]; adds: boolean };

function DesktopFolder({
  uri,
  places,
  applications,
  actions,
  keyboard,
  focusDesktop,
  useDesktopKeyboard = noKeys,
  announce,
  command,
  onState,
}: DesktopProps & { uri: string; places: readonly string[] }) {
  const messages = useShellMessages();
  const platformDrag = usePlatformDrag();
  const hasKeys = useDesktopKeyboard();
  const [items, setItems] = useState<DesktopItem[]>(() => listed(uri));
  const [kept, setKept] = useState<Arrangement>(readKept);
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [selection, setSelection] = useState<string[]>([]);
  const [band, setBand] = useState<{ from: Point; to: Point } | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const area = useRef<ViewRef>(null);
  const origin = useRef<Point>({ x: 0, y: 0 });
  const icons = useRef(new Map<string, React.RefObject<ViewRef | null>>());
  const press = useRef<Press | null>(null);
  const lastPress = useRef<LastPress>(NO_PRESS);
  const anchor = useRef<string | null>(null);
  const ownDrag = useRef<OwnDrag | null>(null);

  // What the folder holds, followed as it changes (Directory.watch, inotify's on OxyOS).
  useEffect(() => {
    const relist = () => setItems(listed(uri));
    relist();
    let watch: { remove(): void } | null = null;
    try {
      watch = new Directory(uri).watch(relist);
    } catch (error) {
      console.warn("the desktop could not watch its folder", { uri, error });
    }
    return () => watch?.remove();
  }, [uri]);

  const width = size?.width ?? 0;
  const grid = useMemo(() => gridOf(width, size?.height ?? 0), [width, size?.height]);
  const names = useMemo(() => items.map((item) => item.name), [items]);
  const shown = useMemo(() => arrange(names, kept, grid), [names, kept, grid]);
  const byName = useMemo(() => new Map(items.map((item) => [item.name, item])), [items]);

  // Each icon where it is shown is kept, once the desktop has its size; an item gone forgets its cell.
  useEffect(() => {
    if (!size) return;
    const next = remembered(shown, kept, grid);
    if (!sameArrangement(next, kept)) setKept(next);
  }, [shown, kept, grid, size]);
  // What is kept is written whenever it changes (an icon dragged, an item dropped, one gone).
  const written = useRef(kept);
  useEffect(() => {
    if (sameArrangement(written.current, kept)) return;
    written.current = kept;
    writeKept(kept);
  }, [kept]);

  // The selection holds only what is there.
  useEffect(() => {
    setSelection((now) => {
      const kept = now.filter((name) => byName.has(name));
      return kept.length === now.length ? now : kept;
    });
  }, [byName]);

  // The top bar's desktop menus.
  const hasSelection = selection.length > 0;
  const hasItems = items.length > 0;
  useEffect(() => {
    onState?.({ keyboard: hasKeys, selection: hasSelection, items: hasItems });
  }, [onState, hasKeys, hasSelection, hasItems]);

  const say = useCallback(
    (text: string) => {
      if (announce) announce(text, false);
      else AccessibilityInfo.announceForAccessibility(text);
    },
    [announce],
  );

  // What acts reads the latest of everything.
  const latest = useRef({ items, byName, shown, selection, grid, width, applications, actions, places });
  latest.current = { items, byName, shown, selection, grid, width, applications, actions, places };

  /** Opens items (decision 5): folders in Dropsea, files with the application that opens them. */
  const open = useCallback(
    (chosen: readonly string[]) => {
      const now = latest.current;
      const opened = chosen.flatMap((name) => {
        const item = now.byName.get(name);
        return item ? [item] : [];
      });
      if (opened.length === 0) return;
      const opening = openingOf(opened, now.applications, uri);
      for (const url of opening.urls) now.actions.openUrl?.(url);
      for (const { application, uris } of opening.files) now.actions.openFiles?.(application, uris);
    },
    [uri],
  );

  /** Puts items in the Trash (decision 5): the session's, with the Trash's sound. */
  const trash = useCallback((chosen: readonly string[]) => {
    const now = latest.current;
    const uris = chosen.flatMap((name) => {
      const item = now.byName.get(name);
      return item ? [item.uri] : [];
    });
    if (uris.length > 0) now.actions.trashFiles?.(uris);
  }, []);

  const selectAll = useCallback(() => {
    const now = latest.current;
    setSelection(inGridOrder(now.shown.keys(), now.shown));
  }, []);

  // The top bar's desktop menus.
  const handled = useRef(0);
  useEffect(() => {
    if (!command || command.id <= handled.current) return;
    handled.current = command.id;
    const chosen = latest.current.selection;
    if (command.action === "open") open(chosen);
    else if (command.action === "trash") trash(chosen);
    else selectAll();
  }, [command, open, trash, selectAll]);

  // The keyboard, while the desktop has it (decision 5): the arrows, Shift extending; Ctrl+O and
  // Ctrl+↓ open, Ctrl+Backspace puts the selection in the Trash, Ctrl+A selects everything, Escape
  // clears the selection. Ctrl is the applications' ⌘ on OxyOS; Super's chords are the system's.
  const keys = useRef(hasKeys);
  keys.current = hasKeys;
  useEffect(() => {
    if (!keyboard) return undefined;
    return keyboard.subscribe((event) => {
      if (!keys.current || event.altKey || event.metaKey) return false;
      const now = latest.current;
      const key = event.key;
      if (event.ctrlKey) {
        if (event.shiftKey) return false;
        if (key.toLowerCase() === "a") {
          selectAll();
          return true;
        }
        if (key.toLowerCase() === "o" || key === "ArrowDown") {
          open(now.selection);
          return true;
        }
        if (key === "Backspace") {
          trash(now.selection);
          return true;
        }
        return false;
      }
      const direction = ARROWS[key];
      if (direction) {
        const moved = arrowed(now.selection, anchor.current, direction, !!event.shiftKey, now.shown);
        anchor.current = moved.anchor;
        setSelection(moved.selection);
        return true;
      }
      if (key === "Escape" && !event.shiftKey && now.selection.length > 0) {
        setSelection([]);
        return true;
      }
      return false;
    });
  }, [keyboard, open, trash, selectAll]);

  const layout = useCallback((event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    setSize((now) => (now && now.width === width && now.height === height ? now : { width, height }));
    area.current?.measureInWindow((x, y) => {
      origin.current = { x, y };
    });
  }, []);

  const pointOf = (event: PointerEvent): Point => ({
    x: event.nativeEvent.clientX - origin.current.x,
    y: event.nativeEvent.clientY - origin.current.y,
  });

  // A press on the desktop gives it the keyboard (decision 5), citing the press, as a press on a
  // Mac's desktop makes Finder active. On an icon it chooses (Shift or Super adding or taking out);
  // a second within half a second opens; on the background it clears the selection and begins a band.
  const pointerDown = useCallback(
    (event: PointerEvent) => {
      const { button, ctrlKey, shiftKey, metaKey } = event.nativeEvent;
      if (button !== 0 || ctrlKey) return;
      focusDesktop?.();
      const now = latest.current;
      const at = pointOf(event);
      const adds = !!shiftKey || !!metaKey;
      const name = itemAt(at, now.shown, now.width);
      if (!name) {
        press.current = { kind: "band", at, base: adds ? [...now.selection] : [], adds };
        lastPress.current = NO_PRESS;
        if (!adds) setSelection([]);
        return;
      }
      const time = pressTime(event);
      const second = !adds && isSecondPress(lastPress.current, name, time);
      lastPress.current = second ? NO_PRESS : { name, at: time };
      const wasSelected = now.selection.includes(name);
      const next = pressed(now.selection, name, adds);
      anchor.current = name;
      setSelection(next);
      const cell = now.shown.get(name);
      const cellAt = cell ? cellOrigin(cell, now.width) : at;
      press.current = second
        ? null
        : { kind: "icon", name, at, grip: { x: at.x - cellAt.x, y: at.y - cellAt.y }, wasSelected, adds };
      if (second) open(next);
    },
    [focusDesktop, open],
  );

  const startDrag = useCallback(
    (name: string, grip: Point) => {
      const now = latest.current;
      if (!platformDrag) return;
      const dragged = inGridOrder(now.selection.includes(name) ? now.selection : [name], now.shown);
      const order = [name, ...dragged.filter((other) => other !== name)];
      const uris = order.flatMap((other) => {
        const item = now.byName.get(other);
        return item ? [item.uri] : [];
      });
      if (uris.length === 0) return;
      const own: OwnDrag = { names: order, gripped: name, grip, live: true };
      ownDrag.current = own;
      // Settled, it is no longer going on; what it was stays known for its drop on the desktop,
      // which may be told after it settled.
      platformDrag
        .start({ data: { files: uris }, actions: DRAG_ACTIONS, previewRef: icons.current.get(name), count: uris.length })
        .catch((error: unknown) => console.warn("the desktop could not drag", { error }))
        .finally(() => {
          own.live = false;
        });
    },
    [platformDrag],
  );

  // Past 4 pixels an icon's press is the platform's drag of the selection (decision 6); the
  // background's draws the band.
  const pointerMove = useCallback(
    (event: PointerEvent) => {
      const now = press.current;
      if (!now) return;
      const at = pointOf(event);
      if (now.kind === "band") {
        if (Math.hypot(at.x - now.at.x, at.y - now.at.y) < DRAG_SLOP && !band) return;
        setBand({ from: now.at, to: at });
        const touched = inBand(now.at, at, latest.current.shown, latest.current.width);
        setSelection(banded(now.base, touched, now.adds));
        return;
      }
      if (Math.hypot(at.x - now.at.x, at.y - now.at.y) < DRAG_SLOP) return;
      press.current = null;
      lastPress.current = NO_PRESS;
      startDrag(now.name, now.grip);
    },
    [band, startDrag],
  );

  // A press on a selected icon let go without a drag, nor Shift or Super: it alone is chosen.
  const pointerUp = useCallback(() => {
    const now = press.current;
    press.current = null;
    setBand(null);
    if (now?.kind === "icon" && now.wasSelected && !now.adds) setSelection([now.name]);
  }, []);

  // The desktop as a target (decision 7): a desktop folder under the pointer is lit, unless it is
  // one of the desktop's own dragged icons.
  const folderAt = (point: Point): string | null => {
    const now = latest.current;
    const name = itemAt(point, now.shown, now.width);
    const item = name ? now.byName.get(name) : undefined;
    if (!item?.folder || (ownDrag.current?.live && ownDrag.current.names.includes(item.name))) return null;
    return item.name;
  };
  const dragOver = useCallback((event: DropTargetDragEvent) => {
    const over = folderAt({ x: event.x, y: event.y });
    setTarget((now) => (now === over ? now : over));
  }, []);
  const dragLeave = useCallback(() => setTarget(null), []);

  /** Files moved or copied into `folder`, each refused where its name is taken (never replaced). */
  const transfer = useCallback(
    async (uris: readonly string[], folder: string, action: "move" | "copy"): Promise<string[]> => {
      const destination = new Directory(folder);
      const taken = new Set(destination.list().map((entry) => entry.name));
      const done: string[] = [];
      for (const each of uris) {
        const name = nameOf(each);
        if (taken.has(name)) {
          say(messages.desktop.taken(name));
          continue;
        }
        const directory = new Directory(each);
        const entry = directory.exists ? directory : new File(each);
        try {
          if (action === "move") await entry.move(destination);
          else await entry.copy(destination);
          taken.add(name);
          done.push(name);
        } catch (error) {
          console.warn(`the desktop could not ${action} an item`, { uri: each, folder, error });
        }
      }
      return done;
    },
    [messages, say],
  );

  /** Copies of the desktop's own items made beside them (Option held), each named as Finder names
      a copy and shown in the cell it was let go in. */
  const duplicate = useCallback(
    async (names: readonly string[], placed: ReadonlyMap<string, Cell>) => {
      const desktop = new Directory(uri);
      const taken = new Set(desktop.list().map((entry) => entry.name));
      const made = new Map<string, Cell>();
      for (const name of names) {
        const item = latest.current.byName.get(name);
        const cell = placed.get(name);
        if (!item || !cell) continue;
        const copy = copyName(name, item.folder, taken, messages.desktop.copy);
        try {
          if (item.folder) await new Directory(item.uri).copy(new Directory(desktop, copy));
          else await new File(item.uri).copy(new File(desktop, copy));
          taken.add(copy);
          made.set(copy, cell);
        } catch (error) {
          console.warn("the desktop could not copy an item", { uri: item.uri, error });
        }
      }
      if (made.size === 0) return;
      setItems(listed(uri));
      setKept((kept) => ({ ...kept, ...Object.fromEntries(made) }));
    },
    [uri, messages],
  );

  const drop = useCallback(
    (event: DropTargetDropEvent) => {
      setTarget(null);
      const now = latest.current;
      const point = { x: event.x, y: event.y };
      const action = event.action === "copy" ? "copy" : "move";
      const uris = event.data.files.flatMap((file) => (file.uri ? [file.uri] : []));
      const onFolder = folderAt(point);
      const folder = onFolder ? (now.byName.get(onFolder)?.uri ?? uri) : uri;
      // The desktop's own items let go on it (from the desktop, or from a window showing its
      // folder): they move to where they were let go, no file moves; with Option, copies of them
      // are made there and they stay, as Finder's. Gripped as the desktop's last drag of them was,
      // else by the middle of the first one's picture.
      const names = uris.map(nameOf);
      if (!onFolder && names.length > 0 && uris.every((each) => sameUri(parentOf(each), uri) && now.byName.has(nameOf(each)))) {
        const own = ownDrag.current;
        const same = own && own.names.length === names.length && own.names.every((name) => names.includes(name));
        const gripped = same ? own.gripped : names[0];
        const grip = same ? own.grip : { x: DESKTOP_CELL.width / 2, y: DESKTOP_PICTURE_BOX / 2 };
        const copying = action === "copy";
        const placed = movedTo(names, gripped, point, grip, now.shown, now.width, now.grid, copying);
        if (copying) void duplicate(names, placed);
        else setKept((kept) => ({ ...kept, ...Object.fromEntries(placed) }));
        return;
      }
      const decided = folderDrop(uris, folder, action, now.places);
      if (decided.kind === "into-itself") {
        say(messages.desktop.intoItself);
        return;
      }
      if (decided.kind === "place") {
        say(messages.desktop.place(nameOf(decided.uri)));
        return;
      }
      if (decided.kind === "nothing") return;
      const placed = onFolder
        ? null
        : droppedAt(decided.uris.map(nameOf), point, now.shown, now.width, now.grid);
      void transfer(decided.uris, folder, action).then((done) => {
        if (!placed || done.length === 0) return;
        // The new items where they were dropped, as the folder is shown with them.
        setItems(listed(uri));
        setKept((kept) => {
          const next: Record<string, Cell> = { ...kept };
          for (const name of done) {
            const cell = placed.get(name);
            if (cell) next[name] = cell;
          }
          return next;
        });
      });
    },
    [uri, messages, say, transfer, duplicate],
  );

  const keysColor = hasKeys;
  const bandRect = band
    ? {
        left: Math.min(band.from.x, band.to.x),
        top: Math.min(band.from.y, band.to.y),
        width: Math.abs(band.to.x - band.from.x),
        height: Math.abs(band.to.y - band.from.y),
      }
    : null;
  return (
    <DropZone
      testID="desktop"
      accept={FILES}
      actions={DROP_ACTIONS}
      folder={target ? (byName.get(target)?.uri ?? uri) : uri}
      onDragEnter={dragOver}
      onDragOver={dragOver}
      onDragLeave={dragLeave}
      onDrop={drop}
      style={FILL}
    >
      <View
        ref={area}
        collapsable={false}
        style={FILL}
        onLayout={layout}
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={pointerUp}
      >
        {size &&
          items.map((item) => {
            const cell = shown.get(item.name);
            if (!cell) return null;
            let ref = icons.current.get(item.name);
            if (!ref) {
              ref = React.createRef<ViewRef>();
              icons.current.set(item.name, ref);
            }
            return (
              <DesktopIcon
                key={item.name}
                item={item}
                cell={cell}
                width={width}
                selected={selection.includes(item.name)}
                lit={target === item.name}
                keyboard={keysColor}
                picture={ref}
                open={open}
                trash={trash}
              />
            );
          })}
        {bandRect && <View pointerEvents="none" style={[BAND, bandRect]} />}
      </View>
    </DropZone>
  );
}

const FILL = { position: "absolute", top: 0, left: 0, right: 0, bottom: 0 } as const;
const BAND = {
  position: "absolute",
  backgroundColor: "rgba(255, 255, 255, 0.18)",
  borderColor: "rgba(255, 255, 255, 0.75)",
  borderWidth: 1,
} as const;

/** The translucent dark square a selected (or lit) picture is drawn on, as a Mac's. */
const PICTURE_SELECTED = "rgba(0, 0, 0, 0.28)";
/** The name's pill while the desktop does not have the keyboard: grey, as a Mac's inactive one. */
const PILL_INACTIVE = "rgba(118, 118, 128, 0.9)";
const NAME = {
  color: "#ffffff",
  fontSize: 12,
  lineHeight: DESKTOP_NAME_LINE,
  fontWeight: "500",
  textAlign: "center",
} as const;
const NAME_SHADOW = {
  textShadowColor: "rgba(0, 0, 0, 0.65)",
  textShadowOffset: { width: 0, height: 1 },
  textShadowRadius: 2,
} as const;

/**
 * One icon (decision 3): its picture over its name, at its cell. Selected, the picture on a dark
 * rounded square and the name on the accent's pill (grey while the desktop does not have the
 * keyboard). An accessible element named by its name, with its kind, whether it is selected, and the
 * actions Open and Move to Trash.
 */
const DesktopIcon = memo(function DesktopIcon({
  item,
  cell,
  width,
  selected,
  lit,
  keyboard,
  picture,
  open,
  trash,
}: {
  item: DesktopItem;
  cell: Cell;
  width: number;
  selected: boolean;
  lit: boolean;
  keyboard: boolean;
  picture: React.RefObject<ViewRef | null>;
  open: (names: readonly string[]) => void;
  trash: (names: readonly string[]) => void;
}) {
  const messages = useShellMessages();
  const kind = kindOf(item);
  const origin = cellOrigin(cell, width);
  const actions = useMemo(
    () => [
      { name: "activate" },
      { name: "open", label: messages.desktop.open },
      { name: "trash", label: messages.desktop.moveToTrash },
    ],
    [messages],
  );
  const act = useCallback(
    (event: AccessibilityActionEvent) => {
      const name = event.nativeEvent.actionName;
      if (name === "activate" || name === "open") open([item.name]);
      else if (name === "trash") trash([item.name]);
    },
    [open, trash, item.name],
  );
  return (
    <View
      ref={picture}
      collapsable={false}
      pointerEvents="none"
      accessible
      accessibilityRole="button"
      accessibilityLabel={item.name}
      accessibilityValue={{ text: messages.desktop.kinds[kind] }}
      accessibilityState={{ selected }}
      accessibilityActions={actions}
      onAccessibilityAction={act}
      testID={`desktop-icon-${item.name}`}
      style={{
        position: "absolute",
        left: origin.x,
        top: origin.y,
        width: DESKTOP_CELL.width,
        alignItems: "center",
      }}
    >
      <View
        style={{
          width: DESKTOP_PICTURE_BOX,
          height: DESKTOP_PICTURE_BOX,
          borderRadius: 6,
          alignItems: "center",
          justifyContent: "center",
          backgroundColor: selected || lit ? PICTURE_SELECTED : "transparent",
        }}
      >
        <Picture item={item} kind={kind} />
      </View>
      <Name name={item.name} selected={selected} keyboard={keyboard} />
    </View>
  );
});

const GLYPHS: Record<Exclude<DesktopKind, "folder" | "image">, typeof RiFileLine> = {
  audio: RiFileMusicLine,
  video: RiFilmLine,
  pdf: RiFilePdf2Line,
  text: RiFileTextLine,
  archive: RiFileZipLine,
  document: RiFileLine,
};

/** Its picture, 64 pixels: a folder's, an image's own (fitted), a document's page with its kind. */
const Picture = memo(function Picture({ item, kind }: { item: DesktopItem; kind: DesktopKind }) {
  const [failed, setFailed] = useState(false);
  if (kind === "folder") {
    return <Image source={FOLDER_ICON} style={PICTURE} resizeMode="contain" accessible={false} />;
  }
  if (kind === "image" && !failed) {
    return (
      <Image
        testID={`desktop-picture-${item.name}`}
        source={{ uri: item.uri }}
        style={PICTURE}
        resizeMode="contain"
        accessible={false}
        onError={() => setFailed(true)}
      />
    );
  }
  return <DocumentIcon glyph={kind === "image" ? RiFileImageLine : GLYPHS[kind]} size={DESKTOP_ICON} />;
});
const PICTURE = { width: DESKTOP_ICON, height: DESKTOP_ICON } as const;

/**
 * Its name (decision 3), white with a shadow on the wallpaper, up to two lines and cut in the
 * middle as Finder cuts it: laid out first, then its first line as it fell and the rest on the
 * second, cut in its middle. Selected, each line on a pill: the accent's while the desktop has the
 * keyboard, grey otherwise.
 */
const Name = memo(function Name({ name, selected, keyboard }: { name: string; selected: boolean; keyboard: boolean }) {
  const [lines, setLines] = useState<{ name: string; lines: string[] } | null>(null);
  const measured = useCallback(
    (event: TextLayoutEvent) => {
      const laid = event.nativeEvent.lines.map((line) => line.text);
      setLines((now) => (now && now.name === name && now.lines.join("\n") === laid.join("\n") ? now : { name, lines: laid }));
    },
    [name],
  );
  const shown = lines && lines.name === name ? lines.lines : null;
  if (!shown) {
    // Laid out once to learn where its lines fall; not drawn.
    return (
      <View style={NAME_BOX}>
        <Text style={[NAME, NAME_HIDDEN]} onTextLayout={measured} accessible={false}>
          {name}
        </Text>
      </View>
    );
  }
  const first = shown.length > 2 ? shown[0].replace(/\s+$/, "") : null;
  const parts = first === null ? shown.map((line) => line.replace(/\s+$/, "")) : [first, name.slice(shown[0].length)];
  return (
    <View style={NAME_BOX}>
      {parts.map((part, index) => (
        <NameLine key={index} text={part} cut={first !== null && index === 1} selected={selected} keyboard={keyboard} />
      ))}
    </View>
  );
});
const NAME_BOX = { width: DESKTOP_NAME_WIDTH, marginTop: 2, alignItems: "center" } as const;
const NAME_HIDDEN = { opacity: 0, width: DESKTOP_NAME_WIDTH } as const;

function NameLine({ text, cut, selected, keyboard }: { text: string; cut: boolean; selected: boolean; keyboard: boolean }) {
  const line = (
    <Text
      numberOfLines={1}
      ellipsizeMode={cut ? "middle" : "clip"}
      accessible={false}
      style={[NAME, selected ? null : NAME_SHADOW, PILL_PADDING]}
    >
      {text}
    </Text>
  );
  if (!selected) return <View style={PILL}>{line}</View>;
  return keyboard ? <AccentPill>{line}</AccentPill> : <View style={[PILL, { backgroundColor: PILL_INACTIVE }]}>{line}</View>;
}
const PILL = { maxWidth: DESKTOP_NAME_WIDTH, borderRadius: 4 } as const;
const PILL_PADDING = { paddingHorizontal: 4 } as const;

/** A line's pill in the person's accent. */
function AccentPill({ children }: { children: React.ReactNode }) {
  return (
    <BloomColorScope colorPreset={useAccent()}>
      <AccentFill>{children}</AccentFill>
    </BloomColorScope>
  );
}
function AccentFill({ children }: { children: React.ReactNode }) {
  const theme = useTheme();
  return <View style={[PILL, { backgroundColor: theme.colors.primary }]}>{children}</View>;
}
