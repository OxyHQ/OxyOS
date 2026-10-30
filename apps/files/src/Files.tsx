/**
 * OxyOS's Files (docs/adr/0027-files.mdx): a Mac's Finder on Bloom. The window is Bloom's
 * AppShell: its sidebar (Favourites, Locations) under the window's controls, and one pane whose
 * toolbar has back and forward, the path, the view and the search. The items are a grid of icons
 * or a list with sortable columns; they are selected with a click, Shift- and Ctrl-clicks and a
 * rubber band, moved by dragging them onto a folder, opened, renamed and put in the Trash from the
 * keyboard, the global menu bar (Bloom's Menubar) and their context menus (Bloom's ContextMenu, a
 * compositor popup on OxyOS). The same on OxyOS and in the browser: only the file system differs
 * (docs/adr/0015-application-model.mdx).
 */
import * as React from "react";
import {
  PanResponder,
  ScrollView,
  TextInput,
  View,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type PointerEvent,
} from "react-native";
import type { ComponentRef } from "react";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { AppShell, useAppShell } from "@oxy.so/bloom/app-shell";
import { BloomScope } from "@oxy.so/bloom/appearance";
import { Breadcrumb, BreadcrumbItem } from "@oxy.so/bloom/breadcrumb";
import { Button } from "@oxy.so/bloom/button";
import { ButtonGroup, ButtonGroupItem } from "@oxy.so/bloom/button-group";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@oxy.so/bloom/context-menu";
import { Dialog } from "@oxy.so/bloom/dialog";
import { EmptyState } from "@oxy.so/bloom/empty-state";
import {
  RiArrowDownSLine,
  RiArrowLeftSLine,
  RiArrowRightSLine,
  RiArrowUpSLine,
  RiComputerLine,
  RiDeleteBinLine,
  RiTvLine,
  RiDownloadLine,
  RiFileTextLine,
  RiFilmLine,
  RiFolderLine,
  RiHardDrive2Line,
  RiHomeLine,
  RiImageLine,
  RiLayoutGridLine,
  RiListView,
  RiMusic2Line,
} from "@oxy.so/bloom/icons";
import { isCommandKey, useWindowKeys, type WindowKeyEvent } from "@oxy.so/bloom/kbd";
import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarRadioGroup,
  MenubarRadioItem,
  MenubarSeparator,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from "@oxy.so/bloom/menubar";
import { Search } from "@oxy.so/bloom/search";
import { SegmentedControl, SegmentedControlItem } from "@oxy.so/bloom/segmented-control";
import type { SidebarTree } from "@oxy.so/bloom/sidebar";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { useWindow, useWindowControlsRoom, WindowDragRegion } from "@oxy.so/bloom/window";
import { FileIcon } from "./FileIcon";
import {
  back as goBack,
  bandSelect,
  baseName,
  boxBetween,
  canDropInto,
  canGoBack,
  canGoForward,
  clickSelect,
  currentLocation,
  FileError,
  focusEnd,
  formatDate,
  formatSize,
  forward as goForward,
  freeName,
  go,
  historyAt,
  isSelected,
  keepShown,
  kindOf,
  moveFocus,
  nameSelection,
  NO_SELECTION,
  parentOf,
  pathParts,
  replace,
  rootOf,
  selectAll,
  selectOnly,
  sortBy,
  sortEntries,
  TYPE_SELECT_PAUSE_MS,
  typeSelect,
  validName,
  visibleEntries,
  type Box,
  type FileEntry,
  type FileErrorCode,
  type FileSystem,
  type History,
  type Location,
  type Places,
  type Selection,
  type SortColumn,
  type SortOrder,
  type TrashedEntry,
} from "./model";
import type { Strings } from "./strings";

export interface FilesProps {
  fs: FileSystem;
  strings: Strings;
  /** The session's locale, for dates. */
  locale: string;
  /** Where the window opens: the home by default. */
  initial?: Location;
}

type ViewMode = "icons" | "list";

/** What the window shows of a location: its items, or why it cannot. */
interface Listing {
  location: Location;
  entries: (FileEntry | TrashedEntry)[];
}

/** Two presses this close on one item are a double click (Bloom's window drag region's, too). */
const DOUBLE_CLICK_MS = 500;
/** A press moved this far is a drag. */
const DRAG_SLOP = 4;
const SIDEBAR_WIDTH = 200;
const FRAME_START = 7;
/** The icon view's cell, and the icon in it. */
const CELL_WIDTH = 104;
const CELL_HEIGHT = 112;
const ICON_SIZE = 64;
const GRID_PADDING = 12;
/** The list's row and its icon. */
const ROW_HEIGHT = 28;
const ROW_ICON = 18;
const HEADER_HEIGHT = 28;

const FAVOURITE_ICONS = {
  home: RiHomeLine,
  desktop: RiTvLine,
  documents: RiFileTextLine,
  downloads: RiDownloadLine,
  pictures: RiImageLine,
  music: RiMusic2Line,
  videos: RiFilmLine,
} as const;

function locationKey(location: Location | null): string {
  if (!location) return "";
  return location.kind === "folder" ? `folder:${location.path}` : location.kind;
}

/** The name the window's title, the path bar and the sidebar give a location. */
function locationName(location: Location, places: Places | null, strings: Strings): string {
  if (location.kind === "trash") return strings.trash;
  if (location.kind === "computer") return strings.computer;
  if (places && location.path === places.home) return strings.favourite.home;
  const favourite = places?.favourites.find((f) => f.path === location.path);
  if (favourite) return strings.favourite[favourite.key];
  const volume = places?.volumes.find((v) => v.path === location.path);
  return volume ? volume.name : baseName(location.path);
}

function errorCode(error: unknown): FileErrorCode {
  return error instanceof FileError ? error.code : "io";
}

export function Files({ fs, strings, locale, initial }: FilesProps) {
  const theme = useTheme();
  const window = useWindow();
  const active = window ? window.active : true;
  const [places, setPlaces] = React.useState<Places | null>(null);
  const [history, setHistory] = React.useState<History | null>(initial ? historyAt(initial) : null);
  const location = history ? currentLocation(history) : null;
  const [listing, setListing] = React.useState<Listing | null>(null);
  const [view, setView] = React.useState<ViewMode>("icons");
  const [order, setOrder] = React.useState<SortOrder>({ column: "name", direction: "ascending" });
  const [search, setSearch] = React.useState("");
  const [showHidden, setShowHidden] = React.useState(false);
  const [selection, setSelection] = React.useState<Selection>(NO_SELECTION);
  const [renaming, setRenaming] = React.useState<string | null>(null);
  const [problem, setProblem] = React.useState<string | null>(null);
  const [confirm, setConfirm] = React.useState<"empty" | "delete" | null>(null);
  const [preview, setPreview] = React.useState<FileEntry | null>(null);
  const [reload, setReload] = React.useState(0);
  const [width, setWidth] = React.useState(0);

  // ---- The places, and where the window opens ----
  const loadPlaces = React.useCallback(() => {
    fs.places().then(setPlaces, (error) => setProblem(strings.errors[errorCode(error)]));
  }, [fs, strings]);
  React.useEffect(() => {
    loadPlaces();
    let stop: (() => void) | null = null;
    let gone = false;
    fs.watch({ places: true }, loadPlaces).then(
      (unwatch) => (gone ? unwatch() : (stop = unwatch)),
      () => {},
    );
    return () => {
      gone = true;
      stop?.();
    };
  }, [fs, loadPlaces]);
  React.useEffect(() => {
    if (places && !history) setHistory(historyAt({ kind: "folder", path: places.home }));
  }, [places, history]);

  // ---- What the location holds, read again when it changes ----
  const key = locationKey(location);
  React.useEffect(() => {
    if (!location) return undefined;
    let current = true;
    const read = async (): Promise<Listing> => {
      if (location.kind === "trash") return { location, entries: await fs.trashList() };
      if (location.kind === "computer") {
        const now = places ?? (await fs.places());
        const roots: FileEntry[] = [now.home, ...now.volumes.map((v) => v.path)].map((path) => ({
          name:
            path === now.home
              ? strings.favourite.home
              : (now.volumes.find((v) => v.path === path)?.name ?? baseName(path)),
          path,
          folder: true,
          link: false,
          size: 0,
          modified: 0,
          hidden: false,
        }));
        return { location, entries: roots };
      }
      const listed = await fs.list(location.path);
      return { location, entries: listed.entries };
    };
    read().then(
      (next) => {
        if (current) setListing(next);
      },
      (error) => {
        if (!current) return;
        // A folder that went: its parent, without a new step in the history.
        if (
          location.kind === "folder" &&
          places &&
          errorCode(error) === "not-found" &&
          rootOf(location.path, places)
        ) {
          setHistory((h) => (h ? replace(h, { kind: "folder", path: parentOf(location.path) }) : h));
        } else {
          setProblem(strings.errors[errorCode(error)]);
        }
      },
    );
    return () => {
      current = false;
    };
    // `location` is named by `key`; the computer's list follows the places.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fs, key, reload, location?.kind === "computer" ? places : null]);

  // The window follows the disk: what changes there, by this window or any other program, is read
  // again when the service says so (decision 6). A place that cannot be watched is read again after
  // each change made here instead.
  const watching = React.useRef(false);
  React.useEffect(() => {
    watching.current = false;
    if (!location || location.kind === "computer") return undefined;
    let stop: (() => void) | null = null;
    let gone = false;
    const target = location.kind === "trash" ? ({ trash: true } as const) : { path: location.path };
    fs.watch(target, () => setReload((n) => n + 1)).then(
      (unwatch) => {
        if (gone) unwatch();
        else {
          stop = unwatch;
          watching.current = true;
        }
      },
      () => {},
    );
    return () => {
      gone = true;
      watching.current = false;
      stop?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fs, key]);

  // A new place starts with nothing selected and no search.
  React.useEffect(() => {
    setSelection(NO_SELECTION);
    setSearch("");
    setRenaming(null);
  }, [key]);

  const inTrash = location?.kind === "trash";
  const atComputer = location?.kind === "computer";
  const shownListing = listing && location && locationKey(listing.location) === key ? listing : null;
  const shown = React.useMemo(
    () =>
      shownListing
        ? sortEntries(
            visibleEntries(shownListing.entries, search, showHidden),
            atComputer ? { column: "name", direction: "ascending" } : order,
          )
        : [],
    [shownListing, search, showHidden, order, atComputer],
  );
  const shownPaths = React.useMemo(() => shown.map((entry) => entry.path), [shown]);
  React.useEffect(() => setSelection((s) => keepShown(s, shownPaths)), [shownPaths]);
  const selected = shown.filter((entry) => isSelected(selection, entry.path));

  // ---- Going places ----
  const navigate = React.useCallback(
    (next: Location) => setHistory((h) => (h ? go(h, next) : historyAt(next))),
    [],
  );
  const open = React.useCallback(
    (entries: readonly FileEntry[]) => {
      const first = entries[0];
      if (!first || inTrash) return;
      if (first.folder) {
        navigate({ kind: "folder", path: first.path });
      } else {
        setProblem(strings.cannotOpen(first.name));
      }
    },
    [inTrash, navigate, strings],
  );
  const goUp = React.useCallback(() => {
    if (!location || location.kind !== "folder" || !places) return;
    const parent = parentOf(location.path);
    if (rootOf(location.path, places)?.path === location.path) {
      navigate({ kind: "computer" });
    } else if (rootOf(parent, places)) {
      navigate({ kind: "folder", path: parent });
    }
  }, [location, places, navigate]);

  // ---- Changes ----
  const report = React.useCallback(
    (results: { ok: boolean; error?: FileErrorCode }[]) => {
      const failed = results.find((result) => !result.ok);
      if (failed && failed.error) setProblem(strings.errors[failed.error]);
    },
    [strings],
  );
  const refresh = React.useCallback(() => {
    if (!watching.current) setReload((n) => n + 1);
  }, []);
  const newFolder = React.useCallback(async () => {
    if (!location || location.kind !== "folder") return;
    const name = freeName(strings.untitledFolder, shownListing?.entries.map((e) => e.name) ?? []);
    try {
      const made = await fs.createFolder(location.path, name);
      refresh();
      setSelection(selectOnly(made.path));
      setRenaming(made.path);
    } catch (error) {
      setProblem(strings.errors[errorCode(error)]);
    }
  }, [fs, location, shownListing, strings, refresh]);
  const moveToTrash = React.useCallback(
    async (entries: readonly FileEntry[]) => {
      if (inTrash || atComputer || entries.length === 0) return;
      report(await fs.trash(entries.map((e) => e.path)));
      refresh();
    },
    [fs, inTrash, atComputer, report, refresh],
  );
  const putBack = React.useCallback(
    async (entries: readonly FileEntry[]) => {
      report(await fs.restore(entries.map((e) => (e as TrashedEntry).id ?? e.path)));
      refresh();
    },
    [fs, report, refresh],
  );
  const deleteForGood = React.useCallback(async () => {
    report(await fs.deleteFromTrash(selected.map((e) => (e as TrashedEntry).id ?? e.path)));
    setConfirm(null);
    refresh();
  }, [fs, selected, report, refresh]);
  const emptyTrash = React.useCallback(async () => {
    try {
      await fs.emptyTrash();
    } catch (error) {
      setProblem(strings.errors[errorCode(error)]);
    }
    setConfirm(null);
    refresh();
    loadPlaces();
  }, [fs, strings, refresh, loadPlaces]);
  const rename = React.useCallback(
    async (path: string, name: string) => {
      setRenaming(null);
      const entry = shown.find((e) => e.path === path);
      if (!entry || name === entry.name) return;
      if (!validName(name)) {
        setProblem(strings.errors["invalid-name"]);
        return;
      }
      try {
        const renamed = await fs.rename(path, name);
        refresh();
        setSelection(selectOnly(renamed.path));
      } catch (error) {
        setProblem(strings.errors[errorCode(error)]);
      }
    },
    [fs, shown, strings, refresh],
  );
  const moveInto = React.useCallback(
    async (paths: readonly string[], folder: string) => {
      report(await fs.move(paths, folder));
      refresh();
    },
    [fs, report, refresh],
  );

  // ---- Keys: arrows, Return, Space, type-select (menus' chords come first, ADR 0021) ----
  const columns = Math.max(1, Math.floor((width - GRID_PADDING * 2) / CELL_WIDTH));
  const typed = React.useRef({ text: "", at: 0 });
  const canRename = selected.length === 1 && !inTrash && !atComputer;
  const onKey = (event: WindowKeyEvent): boolean => {
    if (renaming || confirm || problem) return false;
    const command = isCommandKey(event);
    const step = (by: number) => {
      setSelection((s) => moveFocus(s, shownPaths, by, event.shiftKey));
      return true;
    };
    switch (event.key) {
      case "ArrowDown":
        if (command) {
          open(selected);
          return true;
        }
        return step(view === "list" ? 1 : columns);
      case "ArrowUp":
        if (command) return false;
        return step(view === "list" ? -1 : -columns);
      case "ArrowRight":
        return view === "icons" ? step(1) : false;
      case "ArrowLeft":
        return view === "icons" ? step(-1) : false;
      case "Home":
      case "End":
        setSelection((s) => focusEnd(s, shownPaths, event.key === "Home" ? "first" : "last", event.shiftKey));
        return true;
      case "Enter":
        if (!canRename) return false;
        setRenaming(selected[0].path);
        return true;
      case " ":
        if (command || event.altKey) return false;
        if (preview) setPreview(null);
        else if (selected.length > 0) setPreview(selected[selected.length - 1]);
        return true;
      case "Escape":
        if (preview) {
          setPreview(null);
          return true;
        }
        return false;
      default:
        break;
    }
    // Type-select: a name's first letters, typed within a second of each other.
    if (event.key.length === 1 && !command && !event.altKey && !event.ctrlKey && !event.metaKey) {
      const now = Date.now();
      const text = now - typed.current.at < TYPE_SELECT_PAUSE_MS ? typed.current.text + event.key : event.key;
      typed.current = { text, at: now };
      const found = typeSelect(shown, text);
      if (found) setSelection(selectOnly(found));
      return true;
    }
    return false;
  };
  useWindowKeys(onKey, { enabled: active || !window });
  // Escape in the rename field cancels the rename, as Finder's does; nothing else is taken there.
  useWindowKeys(
    (event) => {
      if (event.key !== "Escape") return false;
      setRenaming(null);
      return true;
    },
    { enabled: renaming !== null, whileTyping: true },
  );

  // ---- Clicks, the rubber band and drags ----
  const lastDown = React.useRef({ path: "", at: 0 });
  const collapseOnRelease = React.useRef<string | null>(null);
  const onItemPointerDown = (entry: FileEntry, event: PointerEvent) => {
    const { button, shiftKey } = event.nativeEvent;
    if (renaming && renaming !== entry.path) setRenaming(null);
    if (button === 2) {
      // A context menu is for the selection, which the item joins if it is not in it. The item's
      // menu opens as the press reaches its trigger, which keeps it from the list's.
      if (!isSelected(selection, entry.path)) setSelection(selectOnly(entry.path));
      return;
    }
    // A click is the item's; the background's deselecting is not for it.
    event.stopPropagation();
    if (button !== 0) return;
    const toggle = isCommandKey(event.nativeEvent);
    const now = Date.now();
    const twice = lastDown.current.path === entry.path && now - lastDown.current.at < DOUBLE_CLICK_MS;
    lastDown.current = { path: entry.path, at: twice ? 0 : now };
    if (twice && !shiftKey && !toggle) {
      open([entry]);
      return;
    }
    // A plain click on an item already selected among others keeps them while it may become a
    // drag; released where it was, it selects that item alone, as Finder's does.
    collapseOnRelease.current =
      !shiftKey && !toggle && isSelected(selection, entry.path) && selection.paths.length > 1
        ? entry.path
        : null;
    setSelection((s) => clickSelect(s, shownPaths, entry.path, { extend: shiftKey, toggle }));
  };
  const onBackgroundPointerDown = (event: PointerEvent) => {
    if (renaming) setRenaming(null);
    const { button, shiftKey } = event.nativeEvent;
    if (button === 0 && !shiftKey && !isCommandKey(event.nativeEvent)) setSelection(NO_SELECTION);
    if (button === 2) setSelection(NO_SELECTION);
  };

  // Where the items are, in the content's own coordinates, from the layout's arithmetic.
  const frames = React.useMemo(() => {
    const map = new Map<string, Box>();
    shown.forEach((entry, index) => {
      if (view === "icons") {
        map.set(entry.path, {
          x: GRID_PADDING + (index % columns) * CELL_WIDTH,
          y: GRID_PADDING + Math.floor(index / columns) * CELL_HEIGHT,
          width: CELL_WIDTH,
          height: CELL_HEIGHT,
        });
      } else {
        map.set(entry.path, { x: 0, y: index * ROW_HEIGHT, width: Math.max(width, 1), height: ROW_HEIGHT });
      }
    });
    return map;
  }, [shown, view, columns, width]);

  // Where the items' view is in the window: measured as it is laid out, at the scroll offset then,
  // and moved by how far it has scrolled since; a press's window point is then a point among the
  // items' frames.
  const content = React.useRef<ComponentRef<typeof View> | null>(null);
  const scrolled = React.useRef(0);
  const origin = React.useRef({ x: 0, y: 0, scrolled: 0 });
  const measureContent = () => {
    const at = scrolled.current;
    content.current?.measureInWindow((x, y) => {
      origin.current = { x, y, scrolled: at };
    });
  };
  const at = (event: GestureResponderEvent) => ({
    x: event.nativeEvent.pageX - origin.current.x,
    y: event.nativeEvent.pageY - (origin.current.y - (scrolled.current - origin.current.scrolled)),
  });
  const [band, setBand] = React.useState<Box | null>(null);
  const bandStart = React.useRef({ x: 0, y: 0, before: NO_SELECTION, adding: false });
  type Drag = { paths: string[]; x: number; y: number; over: string | null };
  const [drag, setDrag] = React.useState<Drag | null>(null);
  // The drag as the responder's handlers see it between renders.
  const dragging = React.useRef<Drag | null>(null);
  const showDrag = (next: Drag | null) => {
    dragging.current = next;
    setDrag(next);
  };
  const live = React.useRef({ shown, frames, selection, shownPaths });
  live.current = { shown, frames, selection, shownPaths };

  const targetAt = (point: { x: number; y: number }, dragged: readonly string[]): string | null => {
    for (const entry of live.current.shown) {
      const frame = live.current.frames.get(entry.path);
      if (
        !frame ||
        point.x < frame.x ||
        point.x > frame.x + frame.width ||
        point.y < frame.y ||
        point.y > frame.y + frame.height
      )
        continue;
      return canDropInto(dragged, entry) ? entry.path : null;
    }
    return null;
  };

  const backgroundResponder = React.useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onPanResponderGrant: (event) => {
          const point = at(event);
          bandStart.current = { ...point, before: live.current.selection, adding: false };
        },
        onPanResponderMove: (event) => {
          const point = at(event);
          const from = bandStart.current;
          if (Math.abs(point.x - from.x) < DRAG_SLOP && Math.abs(point.y - from.y) < DRAG_SLOP) return;
          const box = boxBetween(from, point);
          setBand(box);
          setSelection(
            bandSelect(from.before, live.current.shownPaths, live.current.frames, box, from.adding),
          );
        },
        onPanResponderRelease: () => setBand(null),
        onPanResponderTerminate: () => setBand(null),
        onPanResponderTerminationRequest: () => false,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const itemResponder = React.useCallback(
    (entry: FileEntry) =>
      PanResponder.create({
        // The item takes the press, so the background's band never starts on it.
        onStartShouldSetPanResponder: () => true,
        onPanResponderMove: (event, gesture) => {
          if (!dragging.current && Math.hypot(gesture.dx, gesture.dy) < DRAG_SLOP) return;
          const sel = live.current.selection;
          const paths =
            dragging.current?.paths ?? (isSelected(sel, entry.path) ? sel.paths.slice() : [entry.path]);
          const point = at(event);
          showDrag({ paths, x: point.x, y: point.y, over: targetAt(point, paths) });
        },
        onPanResponderRelease: (event) => {
          const now = dragging.current;
          showDrag(null);
          // Where it was let go decides, even with no move since the last.
          const over = now ? targetAt(at(event), now.paths) : null;
          if (now && over) moveInto(now.paths, over);
          if (!now && collapseOnRelease.current === entry.path) setSelection(selectOnly(entry.path));
          collapseOnRelease.current = null;
        },
        onPanResponderTerminate: () => showDrag(null),
        onPanResponderTerminationRequest: () => false,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [moveInto],
  );

  // ---- Commands, for the menu bar and the context menus ----
  const commands = {
    open: () => open(selected),
    rename: () => canRename && setRenaming(selected[0].path),
    trash: () => moveToTrash(selected),
    putBack: () => putBack(selected),
    deleteNow: () => selected.length > 0 && setConfirm("delete"),
    emptyTrash: () => setConfirm("empty"),
    newFolder: () => newFolder(),
    selectAll: () => setSelection(selectAll(shownPaths)),
  };
  const folderHere = location?.kind === "folder";

  // ---- The sidebar ----
  const sidebarKey = location?.kind === "folder" ? location.path : (location?.kind ?? "");
  const tree: SidebarTree[] = places
    ? [
        {
          key: "favourites",
          label: strings.favourites,
          items: [
            { key: places.home, label: strings.favourite.home, icon: FAVOURITE_ICONS.home },
            ...places.favourites.map((f) => ({
              key: f.path,
              label: strings.favourite[f.key],
              icon: FAVOURITE_ICONS[f.key],
            })),
          ],
        },
        {
          key: "locations",
          label: strings.locations,
          items: [
            { key: "computer", label: strings.computer, icon: RiComputerLine },
            ...places.volumes.map((v) => ({ key: v.path, label: v.name, icon: RiHardDrive2Line })),
            { key: "trash", label: strings.trash, icon: RiDeleteBinLine },
          ],
        },
      ]
    : [];

  const title = location ? locationName(location, places, strings) : strings.title;
  const menus = (
    <Menubar>
      <MenubarMenu value="file">
        <MenubarTrigger>{strings.menus.file}</MenubarTrigger>
        <MenubarContent>
          <MenubarItem shortcut="Mod+Shift+N" disabled={!folderHere} onPress={commands.newFolder}>
            {strings.menus.newFolder}
          </MenubarItem>
          <MenubarItem shortcut="Mod+O" disabled={selected.length === 0 || inTrash} onPress={commands.open}>
            {strings.menus.open}
          </MenubarItem>
          <MenubarSeparator />
          <MenubarItem disabled={!canRename} onPress={commands.rename}>
            {strings.menus.rename}
          </MenubarItem>
          {inTrash ? (
            <>
              <MenubarItem disabled={selected.length === 0} onPress={commands.putBack}>
                {strings.menus.putBack}
              </MenubarItem>
              <MenubarItem
                shortcut="Mod+Alt+Backspace"
                disabled={selected.length === 0}
                onPress={commands.deleteNow}
              >
                {strings.menus.deleteImmediately}
              </MenubarItem>
            </>
          ) : (
            <MenubarItem
              shortcut="Mod+Backspace"
              disabled={selected.length === 0 || atComputer}
              onPress={commands.trash}
            >
              {strings.menus.moveToTrash}
            </MenubarItem>
          )}
          <MenubarSeparator />
          <MenubarItem shortcut="Mod+Shift+Backspace" onPress={commands.emptyTrash}>
            {strings.menus.emptyTrash}
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu value="edit">
        <MenubarTrigger>{strings.menus.edit}</MenubarTrigger>
        <MenubarContent>
          <MenubarItem shortcut="Mod+A" disabled={shown.length === 0} onPress={commands.selectAll}>
            {strings.menus.selectAll}
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu value="view">
        <MenubarTrigger>{strings.menus.view}</MenubarTrigger>
        <MenubarContent>
          <MenubarRadioGroup value={view} onValueChange={(next) => setView(next as ViewMode)}>
            <MenubarRadioItem value="icons" shortcut="Mod+1">
              {strings.asIcons}
            </MenubarRadioItem>
            <MenubarRadioItem value="list" shortcut="Mod+2">
              {strings.asList}
            </MenubarRadioItem>
          </MenubarRadioGroup>
          <MenubarSeparator />
          <MenubarSub>
            <MenubarSubTrigger>{strings.menus.sortBy}</MenubarSubTrigger>
            <MenubarSubContent>
              <MenubarRadioGroup
                value={order.column}
                onValueChange={(column) =>
                  setOrder((o) => (o.column === column ? o : sortBy(o, column as SortColumn)))
                }
              >
                {(["name", "kind", "modified", "size"] as const).map((column) => (
                  <MenubarRadioItem key={column} value={column}>
                    {strings.columns[column]}
                  </MenubarRadioItem>
                ))}
              </MenubarRadioGroup>
            </MenubarSubContent>
          </MenubarSub>
          <MenubarSeparator />
          <MenubarCheckboxItem checked={showHidden} onCheckedChange={setShowHidden} shortcut="Mod+Shift+.">
            {strings.menus.showHidden}
          </MenubarCheckboxItem>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu value="go">
        <MenubarTrigger>{strings.menus.go}</MenubarTrigger>
        <MenubarContent>
          <MenubarItem
            shortcut="Mod+["
            disabled={!history || !canGoBack(history)}
            onPress={() => setHistory((h) => (h ? goBack(h) : h))}
          >
            {strings.back}
          </MenubarItem>
          <MenubarItem
            shortcut="Mod+]"
            disabled={!history || !canGoForward(history)}
            onPress={() => setHistory((h) => (h ? goForward(h) : h))}
          >
            {strings.forward}
          </MenubarItem>
          <MenubarItem shortcut="Mod+ArrowUp" disabled={!folderHere} onPress={goUp}>
            {strings.menus.enclosing}
          </MenubarItem>
          <MenubarSeparator />
          {places ? (
            <>
              <MenubarItem
                shortcut="Mod+Shift+H"
                onPress={() => navigate({ kind: "folder", path: places.home })}
              >
                {strings.favourite.home}
              </MenubarItem>
              {places.favourites.map((f) => (
                <MenubarItem key={f.key} onPress={() => navigate({ kind: "folder", path: f.path })}>
                  {strings.favourite[f.key]}
                </MenubarItem>
              ))}
            </>
          ) : null}
          <MenubarItem shortcut="Mod+Shift+C" onPress={() => navigate({ kind: "computer" })}>
            {strings.computer}
          </MenubarItem>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  );

  return (
    <BloomScope panelRadius={10}>
      <AppShell
        testID="files"
        background="transparent"
        header={null}
        navigationAlign="content"
        navigationGap={0}
        navFrom={0}
        framedFrom={0}
        contentMaxWidth="none"
        frameInset={{ top: 6, end: 7, bottom: 7, start: FRAME_START }}
        sidebar={{
          surface: "plain",
          variant: "panel",
          collapsed: false,
          collapseDisabled: true,
          width: SIDEBAR_WIDTH,
          showSearch: false,
          showThemeToggle: false,
          tree,
          selectedTreeItem: sidebarKey,
          onTreeItemPress: (item) => {
            if (item.key === "computer" || item.key === "trash") navigate({ kind: item.key });
            else navigate({ kind: "folder", path: item.key });
          },
        }}
        panelLayout={{
          mode: "split",
          flow: "bounded",
          panels: [
            {
              id: "browser",
              content: (
                <View style={{ flex: 1 }} testID="browser">
                  {menus}
                  <Toolbar
                    title={title}
                    history={history}
                    location={location}
                    places={places}
                    strings={strings}
                    view={view}
                    search={search}
                    onBack={() => setHistory((h) => (h ? goBack(h) : h))}
                    onForward={() => setHistory((h) => (h ? goForward(h) : h))}
                    onView={setView}
                    onSearch={setSearch}
                    onPath={(path) => navigate({ kind: "folder", path })}
                  />
                  <ContextMenu>
                    <ContextMenuTrigger asChild label={title} style={{ flex: 1, alignSelf: "stretch" }}>
                      <View
                        style={{ flex: 1 }}
                        accessibilityRole={view === "list" ? "list" : "grid"}
                        accessibilityLabel={title}
                        onPointerDown={onBackgroundPointerDown}
                        onLayout={(event: LayoutChangeEvent) => setWidth(event.nativeEvent.layout.width)}
                      >
                        <ScrollView
                          style={{ flex: 1 }}
                          contentContainerStyle={{ flexGrow: 1 }}
                          scrollEventThrottle={16}
                          onScroll={(event) => {
                            scrolled.current = event.nativeEvent.contentOffset.y;
                          }}
                        >
                          {view === "list" ? (
                            <ListHeader
                              order={order}
                              strings={strings}
                              trash={inTrash}
                              onSort={(column) => setOrder((o) => sortBy(o, column))}
                            />
                          ) : null}
                          <View
                            ref={content}
                            onLayout={measureContent}
                            testID="items"
                            style={{ flexGrow: 1, minHeight: 200 }}
                            {...backgroundResponder.panHandlers}
                          >
                            {view === "icons" ? (
                              <View style={{ flexDirection: "row", flexWrap: "wrap", padding: GRID_PADDING }}>
                                {shown.map((entry) => (
                                  <Item
                                    key={entry.path}
                                    entry={entry}
                                    mode="icons"
                                    selected={isSelected(selection, entry.path)}
                                    active={active}
                                    dropTarget={drag?.over === entry.path}
                                    renaming={renaming === entry.path}
                                    volume={atComputer && entry.path !== places?.home}
                                    strings={strings}
                                    locale={locale}
                                    trash={inTrash}
                                    responder={itemResponder}
                                    onPointerDown={onItemPointerDown}
                                    onRename={rename}
                                    onAction={(action) => onItemAction(action, entry)}
                                    menu={itemMenu}
                                  />
                                ))}
                              </View>
                            ) : (
                              shown.map((entry, index) => (
                                <Item
                                  key={entry.path}
                                  entry={entry}
                                  mode="list"
                                  stripe={index % 2 === 1}
                                  selected={isSelected(selection, entry.path)}
                                  active={active}
                                  dropTarget={drag?.over === entry.path}
                                  renaming={renaming === entry.path}
                                  volume={atComputer && entry.path !== places?.home}
                                  strings={strings}
                                  locale={locale}
                                  trash={inTrash}
                                  responder={itemResponder}
                                  onPointerDown={onItemPointerDown}
                                  onRename={rename}
                                  onAction={(action) => onItemAction(action, entry)}
                                  menu={itemMenu}
                                />
                              ))
                            )}
                            {shownListing && shown.length === 0 ? (
                              <View
                                pointerEvents="none"
                                style={{
                                  position: "absolute",
                                  left: 0,
                                  right: 0,
                                  top: 80,
                                  alignItems: "center",
                                }}
                              >
                                <EmptyState
                                  variant="compact"
                                  icon={inTrash ? RiDeleteBinLine : RiFolderLine}
                                  title={
                                    search
                                      ? strings.noResults(search)
                                      : inTrash
                                        ? strings.trashEmpty
                                        : strings.empty
                                  }
                                />
                              </View>
                            ) : null}
                            {band ? (
                              <View
                                pointerEvents="none"
                                style={{
                                  position: "absolute",
                                  left: band.x,
                                  top: band.y,
                                  width: band.width,
                                  height: band.height,
                                  borderWidth: 1,
                                  borderColor: theme.colors.primary,
                                  backgroundColor: theme.isDark
                                    ? "rgba(255,255,255,0.08)"
                                    : "rgba(0,0,0,0.06)",
                                }}
                              />
                            ) : null}
                            {drag ? <DragBadge count={drag.paths.length} x={drag.x} y={drag.y} /> : null}
                          </View>
                        </ScrollView>
                      </View>
                    </ContextMenuTrigger>
                    <ContextMenuContent>
                      {inTrash ? (
                        <ContextMenuItem disabled={shown.length === 0} onPress={commands.emptyTrash}>
                          {strings.menus.emptyTrash}
                        </ContextMenuItem>
                      ) : (
                        <>
                          <ContextMenuItem disabled={!folderHere} onPress={commands.newFolder}>
                            {strings.menus.newFolder}
                          </ContextMenuItem>
                          <ContextMenuSeparator />
                          <ContextMenuItem onPress={() => setView(view === "icons" ? "list" : "icons")}>
                            {`${strings.view} ${view === "icons" ? strings.asList : strings.asIcons}`}
                          </ContextMenuItem>
                        </>
                      )}
                    </ContextMenuContent>
                  </ContextMenu>
                  <StatusBar
                    text={
                      selected.length > 0
                        ? strings.selected(selected.length, shown.length)
                        : strings.items(shown.length)
                    }
                  />
                </View>
              ),
            },
          ],
        }}
      />
      <AlertDialog
        open={problem !== null}
        onOpenChange={(open) => !open && setProblem(null)}
        title={problem ?? ""}
        confirmLabel={strings.ok}
        hideCancel
        onConfirm={() => setProblem(null)}
        testID="files-problem"
      />
      <AlertDialog
        open={confirm !== null}
        onOpenChange={(open) => !open && setConfirm(null)}
        title={
          confirm === "empty"
            ? strings.confirmEmpty.title(shownListing && inTrash ? shown.length : (places?.trash ?? 0))
            : strings.confirmDelete.title(selected.length)
        }
        description={strings.confirmEmpty.body}
        confirmLabel={confirm === "empty" ? strings.confirmEmpty.confirm : strings.confirmDelete.confirm}
        cancelLabel={strings.confirmEmpty.cancel}
        destructive
        onConfirm={() => (confirm === "empty" ? emptyTrash() : deleteForGood())}
        onCancel={() => setConfirm(null)}
        testID="files-confirm"
      />
      <Dialog
        open={preview !== null}
        onClose={() => setPreview(null)}
        title={preview?.name}
        testID="files-preview"
      >
        {preview ? <Preview entry={preview} strings={strings} locale={locale} /> : null}
      </Dialog>
    </BloomScope>
  );

  function itemMenu(entry: FileEntry) {
    return inTrash ? (
      <>
        <ContextMenuItem onPress={commands.putBack}>{strings.menus.putBack}</ContextMenuItem>
        <ContextMenuItem variant="destructive" onPress={commands.deleteNow}>
          {strings.menus.deleteImmediately}
        </ContextMenuItem>
      </>
    ) : (
      <>
        <ContextMenuItem onPress={() => open(isSelected(selection, entry.path) ? selected : [entry])}>
          {strings.menus.open}
        </ContextMenuItem>
        {atComputer ? null : (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem disabled={selected.length > 1} onPress={() => setRenaming(entry.path)}>
              {strings.menus.rename}
            </ContextMenuItem>
            <ContextMenuItem onPress={commands.trash}>{strings.menus.moveToTrash}</ContextMenuItem>
          </>
        )}
      </>
    );
  }

  function onItemAction(action: string, entry: FileEntry) {
    // The screen reader's actions on an item (ADR 0003).
    if (action === "activate") open([entry]);
    else if (action === "select")
      setSelection((s) => clickSelect(s, shownPaths, entry.path, { toggle: true }));
    else if (action === "rename" && !inTrash && !atComputer) {
      setSelection(selectOnly(entry.path));
      setRenaming(entry.path);
    } else if (action === "trash") {
      if (inTrash) putBack([entry]);
      else moveToTrash(isSelected(selection, entry.path) ? selected : [entry]);
    }
  }
}

// ---- The toolbar ----

function Toolbar({
  title,
  history,
  location,
  places,
  strings,
  view,
  search,
  onBack,
  onForward,
  onView,
  onSearch,
  onPath,
}: {
  title: string;
  history: History | null;
  location: Location | null;
  places: Places | null;
  strings: Strings;
  view: ViewMode;
  search: string;
  onBack: () => void;
  onForward: () => void;
  onView: (view: ViewMode) => void;
  onSearch: (text: string) => void;
  onPath: (path: string) => void;
}) {
  // With the sidebar hidden the toolbar holds the window's corner, so it keeps the controls' room.
  const room = useWindowControlsRoom();
  const { navigationDock } = useAppShell();
  const parts = location?.kind === "folder" && places ? pathParts(location.path, places) : [];
  return (
    <WindowDragRegion
      style={{
        height: 52,
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        paddingInlineStart: navigationDock.hidden ? Math.max(8, room.leading - FRAME_START) : 8,
        paddingInlineEnd: 10,
      }}
      accessibilityRole="toolbar"
    >
      <ButtonGroup size="sm" accessibilityLabel={strings.title}>
        <ButtonGroupItem
          iconOnly
          leadingIcon={RiArrowLeftSLine}
          accessibilityLabel={strings.back}
          disabled={!history || !canGoBack(history)}
          onPress={onBack}
          testID="back"
        />
        <ButtonGroupItem
          iconOnly
          leadingIcon={RiArrowRightSLine}
          accessibilityLabel={strings.forward}
          disabled={!history || !canGoForward(history)}
          onPress={onForward}
          testID="forward"
        />
      </ButtonGroup>
      <View style={{ flex: 1, minWidth: 72 }}>
        {parts.length > 1 ? (
          <Breadcrumb size="sm" overflow="collapse" accessibilityLabel={strings.path} testID="path">
            {parts.map((part, index) => (
              <BreadcrumbItem
                key={part.path}
                current={index === parts.length - 1}
                onPress={index === parts.length - 1 ? undefined : () => onPath(part.path)}
              >
                {index === 0 && places && part.path === places.home ? strings.favourite.home : part.name}
              </BreadcrumbItem>
            ))}
          </Breadcrumb>
        ) : (
          <Text variant="headline-semibold" numberOfLines={1} accessibilityRole="header" testID="title">
            {title}
          </Text>
        )}
      </View>
      <SegmentedControl
        type="radio"
        size="sm"
        label={strings.view}
        value={view}
        onValueChange={(next) => onView(next as ViewMode)}
      >
        <SegmentedControlItem value="icons" accessibilityLabel={strings.asIcons} testID="view-icons">
          <IconGlyph Icon={RiLayoutGridLine} />
        </SegmentedControlItem>
        <SegmentedControlItem value="list" accessibilityLabel={strings.asList} testID="view-list">
          <IconGlyph Icon={RiListView} />
        </SegmentedControlItem>
      </SegmentedControl>
      {/* The search gives way first in a narrow window, as Finder's does; the path keeps a little. */}
      <View style={{ width: 190, minWidth: 110, flexShrink: 1 }}>
        <Search
          size="sm"
          value={search}
          onChangeText={onSearch}
          onClearText={() => onSearch("")}
          placeholder={strings.searchIn(title)}
          accessibilityLabel={strings.search}
          testID="search"
        />
      </View>
    </WindowDragRegion>
  );
}

function IconGlyph({
  Icon,
}: {
  Icon: React.ComponentType<{ width?: number; height?: number; fill?: string }>;
}) {
  const theme = useTheme();
  return <Icon width={16} height={16} fill={theme.colors.text} />;
}

// ---- The list's columns ----

function ListHeader({
  order,
  strings,
  trash,
  onSort,
}: {
  order: SortOrder;
  strings: Strings;
  trash: boolean;
  onSort: (column: SortColumn) => void;
}) {
  const theme = useTheme();
  const column = (id: SortColumn, label: string, width: number | null) => {
    const sorted = order.column === id;
    return (
      <View key={id} style={width === null ? { flex: 1, minWidth: 0 } : { width }}>
        <Button
          size="xs"
          tone="neutral"
          appearance="plain"
          trailingIcon={
            sorted ? (order.direction === "ascending" ? RiArrowUpSLine : RiArrowDownSLine) : undefined
          }
          onPress={() => onSort(id)}
          accessibilityLabel={
            sorted
              ? `${label}, ${order.direction === "ascending" ? strings.ascending : strings.descending}`
              : label
          }
          testID={`column-${id}`}
          style={{ alignSelf: "flex-start" }}
        >
          {label}
        </Button>
      </View>
    );
  };
  return (
    <View
      style={{
        height: HEADER_HEIGHT,
        flexDirection: "row",
        alignItems: "center",
        // The rows' own inset, so each header's label sits over its column's values.
        marginHorizontal: 6,
        paddingHorizontal: 8,
        borderBottomWidth: 1,
        borderBottomColor: theme.colors.borderLight,
      }}
    >
      {column("name", strings.columns.name, null)}
      {column("modified", trash ? strings.columns.deleted : strings.columns.modified, 190)}
      {column("size", strings.columns.size, 90)}
      {column("kind", strings.columns.kind, 140)}
    </View>
  );
}

// ---- An item ----

function Item({
  entry,
  mode,
  stripe,
  selected,
  active,
  dropTarget,
  renaming,
  volume,
  strings,
  locale,
  trash,
  responder,
  onPointerDown,
  onRename,
  onAction,
  menu,
}: {
  entry: FileEntry;
  mode: ViewMode;
  stripe?: boolean;
  selected: boolean;
  active: boolean;
  dropTarget: boolean;
  renaming: boolean;
  volume: boolean;
  strings: Strings;
  locale: string;
  trash: boolean;
  responder: (entry: FileEntry) => ReturnType<typeof PanResponder.create>;
  onPointerDown: (entry: FileEntry, event: PointerEvent) => void;
  onRename: (path: string, name: string) => void;
  onAction: (action: string) => void;
  menu: (entry: FileEntry) => React.ReactNode;
}) {
  const theme = useTheme();
  const colours = theme.colors;
  const handlers = React.useMemo(() => responder(entry), [responder, entry]);
  const kind = volume ? strings.volume : strings.kinds[kindOf(entry)];
  const size = entry.folder ? "--" : formatSize(entry.size, strings.units);
  const when = (entry as TrashedEntry).deleted ?? entry.modified;
  const date = when ? formatDate(when, Date.now(), locale, strings) : "--";
  // Selected: the accent in the active window, grey in an inactive one, as Finder's.
  const fill = selected
    ? active
      ? colours.primary
      : theme.isDark
        ? "#4a4a50"
        : "#dcdce0"
    : dropTarget
      ? colours.primarySubtle
      : undefined;
  const ink = selected && active ? colours.primaryForeground : colours.text;
  const secondary = selected && active ? colours.primaryForeground : colours.textSecondary;
  const accessibility = {
    accessible: true,
    accessibilityRole: "listitem" as const,
    accessibilityLabel: entry.name,
    accessibilityValue: { text: mode === "list" ? `${kind}, ${size}, ${date}` : kind },
    accessibilityState: { selected },
    accessibilityActions: [
      { name: "activate", label: strings.menus.open },
      { name: "select", label: strings.select },
      ...(volume ? [] : [{ name: "rename", label: strings.menus.rename }]),
      ...(volume
        ? []
        : [{ name: "trash", label: trash ? strings.menus.putBack : strings.menus.moveToTrash }]),
    ],
    onAccessibilityAction: (event: { nativeEvent: { actionName: string } }) =>
      onAction(event.nativeEvent.actionName),
  };
  const name = renaming ? (
    <RenameField
      entry={entry}
      centred={mode === "icons"}
      onDone={(next) => onRename(entry.path, next)}
      strings={strings}
    />
  ) : null;
  const body =
    mode === "icons" ? (
      <View
        testID={`item-${entry.name}`}
        {...accessibility}
        {...handlers.panHandlers}
        onPointerDown={(event: PointerEvent) => onPointerDown(entry, event)}
        style={{ width: CELL_WIDTH, height: CELL_HEIGHT, alignItems: "center", paddingTop: 6, gap: 4 }}
      >
        <View
          style={{
            padding: 4,
            borderRadius: 8,
            backgroundColor: selected
              ? theme.isDark
                ? "rgba(255,255,255,0.14)"
                : "rgba(0,0,0,0.08)"
              : dropTarget
                ? colours.primarySubtle
                : undefined,
          }}
        >
          <FileIcon entry={entry} size={ICON_SIZE} volume={volume} />
        </View>
        {name ?? (
          <View
            style={{
              maxWidth: CELL_WIDTH - 8,
              borderRadius: 5,
              paddingHorizontal: 5,
              paddingVertical: 1,
              backgroundColor: selected ? fill : undefined,
            }}
          >
            <Text variant="caption-1-regular" numberOfLines={2} style={{ textAlign: "center", color: ink }}>
              {entry.name}
            </Text>
          </View>
        )}
      </View>
    ) : (
      <View
        testID={`item-${entry.name}`}
        {...accessibility}
        {...handlers.panHandlers}
        onPointerDown={(event: PointerEvent) => onPointerDown(entry, event)}
        style={{
          height: ROW_HEIGHT,
          flexDirection: "row",
          alignItems: "center",
          paddingHorizontal: 8,
          marginHorizontal: 6,
          borderRadius: 6,
          backgroundColor:
            fill ?? (stripe ? (theme.isDark ? "rgba(255,255,255,0.03)" : "rgba(0,0,0,0.025)") : undefined),
        }}
      >
        <View style={{ flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: 8 }}>
          <FileIcon entry={entry} size={ROW_ICON} volume={volume} thumbnail={false} />
          {name ?? (
            <Text variant="body-2-regular" numberOfLines={1} style={{ flex: 1, color: ink }}>
              {entry.name}
            </Text>
          )}
        </View>
        {/* Each value under its header's label (a button's 8-point inset). */}
        <Text
          variant="body-2-regular"
          numberOfLines={1}
          style={{ width: 190, paddingStart: 8, color: secondary }}
        >
          {date}
        </Text>
        <Text
          variant="body-2-regular"
          numberOfLines={1}
          style={{ width: 90, paddingStart: 8, color: secondary }}
        >
          {size}
        </Text>
        <Text
          variant="body-2-regular"
          numberOfLines={1}
          style={{ width: 140, paddingStart: 8, color: secondary }}
        >
          {kind}
        </Text>
      </View>
    );
  return (
    <ContextMenu>
      <ContextMenuTrigger
        asChild
        label={entry.name}
        style={mode === "list" ? { alignSelf: "stretch" } : undefined}
      >
        {body}
      </ContextMenuTrigger>
      <ContextMenuContent>{menu(entry)}</ContextMenuContent>
    </ContextMenu>
  );
}

/** The name being edited in place: its stem selected first, Return or leaving it keeps it. */
function RenameField({
  entry,
  centred,
  onDone,
  strings,
}: {
  entry: FileEntry;
  centred: boolean;
  onDone: (name: string) => void;
  strings: Strings;
}) {
  const theme = useTheme();
  const [text, setText] = React.useState(entry.name);
  const input = React.useRef<ComponentRef<typeof TextInput> | null>(null);
  const done = React.useRef(false);
  const finish = () => {
    if (done.current) return;
    done.current = true;
    onDone(text);
  };
  return (
    <TextInput
      testID="rename"
      autoFocus
      ref={input}
      value={text}
      onChangeText={setText}
      // The name without its extension selected first, as Finder selects it.
      onFocus={() => {
        const { start, end } = nameSelection(entry.name, entry.folder);
        input.current?.setSelection(start, end);
      }}
      onSubmitEditing={finish}
      onBlur={finish}
      accessibilityLabel={strings.rename}
      style={{
        minWidth: centred ? CELL_WIDTH - 8 : 120,
        maxWidth: centred ? CELL_WIDTH - 4 : undefined,
        flex: centred ? undefined : 1,
        textAlign: centred ? "center" : "left",
        fontSize: centred ? 12 : 13,
        paddingHorizontal: 4,
        paddingVertical: 1,
        borderWidth: 2,
        borderColor: theme.colors.primary,
        borderRadius: 4,
        color: theme.colors.text,
        backgroundColor: theme.colors.background,
      }}
    />
  );
}

function DragBadge({ count, x, y }: { count: number; x: number; y: number }) {
  const theme = useTheme();
  return (
    <View
      pointerEvents="none"
      style={{
        position: "absolute",
        left: x + 8,
        top: y + 8,
        minWidth: 24,
        height: 24,
        borderRadius: 12,
        paddingHorizontal: 8,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: theme.colors.primary,
      }}
    >
      <Text variant="caption-1-semibold" style={{ color: theme.colors.primaryForeground }}>
        {String(count)}
      </Text>
    </View>
  );
}

function StatusBar({ text }: { text: string }) {
  const theme = useTheme();
  return (
    <View
      style={{
        height: 26,
        alignItems: "center",
        justifyContent: "center",
        borderTopWidth: 1,
        borderTopColor: theme.colors.borderLight,
      }}
    >
      <Text
        variant="caption-1-regular"
        style={{ color: theme.colors.textSecondary }}
        testID="status"
        accessibilityLiveRegion="polite"
      >
        {text}
      </Text>
    </View>
  );
}

/** Space's preview: the picture, or the item's icon and what it is. */
function Preview({ entry, strings, locale }: { entry: FileEntry; strings: Strings; locale: string }) {
  const theme = useTheme();
  return (
    <View style={{ alignItems: "center", gap: 12, padding: 16 }}>
      <FileIcon entry={entry} size={entry.folder ? 128 : 256} />
      <Text variant="body-2-regular" style={{ color: theme.colors.textSecondary }}>
        {`${strings.kinds[kindOf(entry)]} · ${entry.folder ? "--" : formatSize(entry.size, strings.units)}`}
      </Text>
      <Text variant="caption-1-regular" style={{ color: theme.colors.textSecondary }}>
        {`${strings.modifiedLabel}: ${entry.modified ? formatDate(entry.modified, Date.now(), locale, strings) : "--"}`}
      </Text>
    </View>
  );
}
