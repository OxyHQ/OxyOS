import React, { memo, useCallback, useEffect, useRef, useState, type ComponentRef } from "react";
import {
  AccessibilityInfo,
  StyleSheet,
  View,
  useColorScheme,
  useWindowDimensions,
  type PointerEvent,
} from "react-native";
import { BlurTargetView, BlurView } from "expo-blur";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { BloomColorScope } from "@oxy.so/bloom/theme";
import { Search } from "@oxy.so/bloom/search";
import { usePlatformDrag } from "@oxy.so/bloom/platform-surface";
import { APPLICATION_TYPE, DRAG_SLOP, shellDrag } from "../app-drag";
import { Box, PressArea } from "../primitives";
import { AppIcon } from "../apps";
import { Wallpaper } from "./Wallpaper";
import type { ShellWallpaper } from "../wallpaper";
import type { ShellApplication, ShellFolder } from "../model";
import type { ShellKeyboard } from "../platform";
import {
  HIGHLIGHT_PADDING,
  SEARCH_HEIGHT,
  launcherLayout,
  moveSelection,
  pageOf,
  type LauncherKey,
} from "../launcher";
import { useShellMessages } from "../useShellMessages";

const normalize = (value: string) =>
  value
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase();
type Entry = { app: ShellApplication } | { folder: ShellFolder };

/** The search field's colours over the blurred desktop. One object for every render: Bloom derives
    the scope's theme from it again whenever it is another. */
const SEARCH_TOKENS = {
  background: "#00000000",
  surface: "#ffffff1f",
  foreground: "#ffffff",
  // A soft focus ring, as Launchpad's field has: the caret says it has the keyboard.
  primary: "#ffffff66",
  border: "#ffffff33",
};

/**
 * An application's icon in Launchpad that drags out of it (docs/adr/0034-the-dock.mdx, decision 10):
 * a mouse press moved past the slop drags the application, its icon as it looks the image, as a
 * Mac's Launchpad drags an icon onto the Dock, which keeps it where it is dropped. Launchpad stays
 * open; a drop anywhere else does nothing and the image goes back.
 */
const DraggableAppIcon = memo(function DraggableAppIcon({ app, size }: { app: ShellApplication; size: number }) {
  const platformDrag = usePlatformDrag();
  const picture = useRef<ComponentRef<typeof View>>(null);
  const pressedAt = useRef<{ x: number; y: number } | null>(null);
  const onPointerDown = useCallback((event: PointerEvent) => {
    const { button, ctrlKey, clientX, clientY } = event.nativeEvent;
    pressedAt.current = button === 0 && !ctrlKey ? { x: clientX, y: clientY } : null;
  }, []);
  const onPointerMove = useCallback(
    (event: PointerEvent) => {
      const at = pressedAt.current;
      if (!at || !platformDrag) return;
      const { clientX, clientY } = event.nativeEvent;
      if (Math.hypot(clientX - at.x, clientY - at.y) < DRAG_SLOP) return;
      pressedAt.current = null;
      shellDrag.current = app.id;
      platformDrag
        .start({ data: { other: { [APPLICATION_TYPE]: app.id } }, actions: ["copy"], previewRef: picture })
        .catch((error: unknown) => console.warn("Launchpad could not drag", { id: app.id, error }))
        .finally(() => {
          if (shellDrag.current === app.id) shellDrag.current = null;
        });
    },
    [platformDrag, app.id],
  );
  const onPointerUp = useCallback(() => {
    pressedAt.current = null;
  }, []);
  return (
    <View
      ref={picture}
      collapsable={false}
      onPointerDownCapture={platformDrag ? onPointerDown : undefined}
      onPointerMove={platformDrag ? onPointerMove : undefined}
      onPointerUp={platformDrag ? onPointerUp : undefined}
      onPointerCancel={platformDrag ? onPointerUp : undefined}
    >
      <AppIcon app={app} size={size} />
    </View>
  );
});

/** Moves the screen reader's focus to `view` (React Native's accessibility focus). */
function focusForAccessibility(view: ComponentRef<typeof View> | null | undefined) {
  if (view) {
    AccessibilityInfo.sendAccessibilityEvent(
      view as Parameters<typeof AccessibilityInfo.sendAccessibilityEvent>[0],
      "focus",
    );
  }
}

/** Read by the screen reader, drawn nowhere: Launchpad shows no count. */
const SAID_ONLY = {
  position: "absolute",
  width: 1,
  height: 1,
  overflow: "hidden",
  opacity: 0,
} as const;

const LABEL_SHADOW = {
  textShadowColor: "#00000059",
  textShadowOffset: { width: 0, height: 1 },
  textShadowRadius: 2,
};

/**
 * Launchpad's backdrop, a surface of its own beneath its icons (docs/adr/0011-transitions.mdx):
 * the desktop's wallpaper, at the screen's size so it meets the desktop's exactly, blurred and
 * darkened as a Mac's Launchpad blurs the desktop. The compositor fades it in and out, never
 * scaling it, while the icons above settle.
 */
export const LaunchpadBackdrop = memo(function LaunchpadBackdrop({
  open,
  wallpaper,
}: {
  /** Whether it is shown, with Launchpad. */
  open: boolean;
  wallpaper: ShellWallpaper;
}) {
  const { width, height } = useWindowDimensions();
  const dark = useColorScheme() === "dark";
  const target = useRef<ComponentRef<typeof View>>(null);
  return (
    <Box
      testID="shell-launchpad-backdrop"
      pointerEvents="none"
      className="absolute inset-0 overflow-hidden"
      style={{ display: open ? "flex" : "none" }}
    >
      <BlurTargetView
        ref={target}
        style={{ position: "absolute", left: 0, top: 0, width, height }}
      >
        <Wallpaper wallpaper={wallpaper} decorative />
      </BlurTargetView>
      {/* A blur of radius 30 (its intensity over its reduction factor), and a dark tint over it
          (a fifth; a quarter in dark mode): the icons' white labels read on either. */}
      <BlurView
        blurTarget={target}
        blurMethod="dimezisBlurView"
        intensity={36}
        blurReductionFactor={1.2}
        tint={dark ? "dark" : "systemUltraThinMaterialDark"}
        style={StyleSheet.absoluteFill}
      />
    </Box>
  );
});

/**
 * Launchpad, as a Mac's: the applications in pages of a grid over the blurred desktop, the
 * search above, the pages' dots below. Typing searches (the field takes the keyboard as it
 * opens); the arrows move a highlight through the icons and Return opens it; Escape clears the
 * search, then closes; a press outside the icons, or opening one, closes it. The highlight is the
 * screen reader's focus, as VoiceOver's cursor is Launchpad's selection: each icon the arrows reach
 * is said once by its name; typing again gives the search its focus back. An open folder is modal:
 * only it is read and reached with Tab, as VoiceOver reads only an open Launchpad folder. It opens and closes
 * by the compositor's motion (its icons settle from larger over a fading backdrop,
 * docs/adr/0011-transitions.mdx), on the display's time, not React's.
 */
export const Launchpad = memo(function Launchpad({
  open,
  launch,
  dismiss,
  applications,
  folders,
  escapeRequest = 0,
  searchRequest = 0,
  keyboard,
}: {
  /** Whether it is shown. It stays mounted while hidden, as macOS keeps Launchpad, so showing it
      draws what is already built instead of building it then. */
  open: boolean;
  launch: (app: string) => void;
  applications: readonly ShellApplication[];
  folders: readonly ShellFolder[];
  dismiss: () => void;
  escapeRequest?: number;
  /** A new value focuses the search, so what is typed searches (Super pressed alone, ADR 0041,
      docs/adr/0024-system-shortcuts.mdx). */
  searchRequest?: number;
  /** The platform's keys before the focused field, where it has them (the arrows and Return). */
  keyboard?: ShellKeyboard;
}) {
  const messages = useShellMessages();
  const words = messages.launchpad;
  const [query, setQuery] = useState("");
  const [folder, setFolder] = useState<ShellFolder | null>(null);
  const [page, setPage] = useState(0);
  // The icon highlighted by the arrows (an index into every entry), none until one is pressed;
  // with a search, its first result.
  const [selected, setSelected] = useState<number | null>(null);
  // The icons on the page, by entry, and the folder's panel: where the screen reader's focus goes.
  const icons = useRef(new Map<string, ComponentRef<typeof View>>());
  const folderPanel = useRef<ComponentRef<typeof View>>(null);
  // Whether the arrows moved the screen reader's focus off the search, to give it back on typing.
  const arrowed = useRef(false);
  // The arrows asked for the highlight to take the screen reader's focus.
  const [focusRequest, setFocusRequest] = useState(0);
  const { width, height } = useWindowDimensions();
  const layout = launcherLayout(width, height);
  const { columns, rows, gridWidth, iconSize, rowHeight, labelSize } = layout;
  const capacity = columns * rows;
  const grouped = new Set(folders.flatMap((group) => [...group.appIds]));
  const entries: Entry[] = applications
    .filter(
      (app) =>
        (query || !grouped.has(app.id)) &&
        normalize(app.name).includes(normalize(query)),
    )
    .map((app) => ({ app }));
  if (!query) entries.push(...folders.map((group) => ({ folder: group })));
  const pageCount = Math.max(1, Math.ceil(entries.length / capacity));
  const currentPage = Math.min(page, pageCount - 1);
  const keyOf = (entry: Entry) =>
    "app" in entry ? entry.app.id : `folder-${entry.folder.id}`;
  const openEntry = (entry: Entry | undefined) => {
    if (!entry) return;
    if ("app" in entry) launch(entry.app.id);
    else setFolder(entry.folder);
  };

  const search = (value: string) => {
    // Typing after the arrows: the search has the screen reader's focus again, so what is typed
    // is echoed, as VoiceOver goes back to the field.
    if (arrowed.current) {
      arrowed.current = false;
      focusForAccessibility(field.current as unknown as ComponentRef<typeof View>);
    }
    setQuery(value);
    setPage(0);
    setSelected(value ? 0 : null);
  };
  const close = () => {
    if (folder) setFolder(null);
    else if (query) search("");
    else dismiss();
  };
  // What a key does, wherever it comes from (the platform's keys first, else the field's).
  const onKey = (key: string): boolean => {
    if (key === "Escape") {
      close();
      return true;
    }
    if (folder) return false;
    if (key === "Enter") {
      if (selected === null) return false;
      openEntry(entries[selected]);
      return true;
    }
    if (
      key === "ArrowLeft" ||
      key === "ArrowRight" ||
      key === "ArrowUp" ||
      key === "ArrowDown"
    ) {
      const next = moveSelection(
        selected,
        key as LauncherKey,
        entries.length,
        columns,
        rows,
        currentPage,
      );
      setSelected(next);
      if (next !== null) {
        setPage(pageOf(next, columns, rows));
        arrowed.current = true;
        setFocusRequest((request) => request + 1);
      }
      return true;
    }
    return false;
  };
  const latestKey = useRef(onKey);
  latestKey.current = onKey;
  useEffect(() => {
    if (!open || !keyboard) return undefined;
    return keyboard.subscribe((event) =>
      event.altKey || event.ctrlKey || event.metaKey
        ? false
        : latestKey.current(event.key),
    );
  }, [open, keyboard]);

  const latestClose = useRef(close);
  latestClose.current = close;
  const lastEscape = useRef(escapeRequest);
  useEffect(() => {
    if (open && lastEscape.current !== escapeRequest) latestClose.current();
    lastEscape.current = escapeRequest;
  }, [escapeRequest, open]);
  const field = useRef<React.ComponentRef<typeof Search>>(null);
  // The icon the arrows reached takes the screen reader's focus once it is drawn (its page may be
  // another), which says it.
  const highlighted = selected === null ? undefined : entries[selected];
  const highlightedKey = highlighted ? keyOf(highlighted) : undefined;
  useEffect(() => {
    if (!open || focusRequest === 0 || !highlightedKey) return;
    focusForAccessibility(icons.current.get(highlightedKey));
  }, [focusRequest]);
  // A folder opened takes the screen reader into it, at its name; closed, the reader is back on
  // its icon where the arrows had brought it.
  const lastFolder = useRef<ShellFolder | null>(null);
  useEffect(() => {
    const closed = lastFolder.current;
    lastFolder.current = folder;
    if (!open) return;
    if (folder) focusForAccessibility(folderPanel.current);
    else if (closed && arrowed.current) {
      focusForAccessibility(icons.current.get(`folder-${closed.id}`));
    }
  }, [folder, open]);
  const lastSearch = useRef(searchRequest);
  // Shown, the search takes what is typed, as Launchpad's does, and the screen reader's focus as
  // it has the keyboard's: Launchpad said by its title as it appears, then its search, as
  // VoiceOver says them. Super alone asks again.
  useEffect(() => {
    if (!open) return;
    field.current?.focus();
    focusForAccessibility(field.current as unknown as ComponentRef<typeof View>);
  }, [open]);
  useEffect(() => {
    if (open && lastSearch.current !== searchRequest) field.current?.focus();
    lastSearch.current = searchRequest;
  }, [searchRequest, open]);
  // Closed, it starts again where it opens: no search, no folder, the first page; and its field
  // lets the keyboard go, so the next opening asks for it again.
  useEffect(() => {
    if (open) return;
    field.current?.blur();
    setQuery("");
    setFolder(null);
    setPage(0);
    setSelected(null);
    arrowed.current = false;
  }, [open]);
  const folderWidth = Math.min(640, width - 64);
  const folderColumns = width >= 700 ? 5 : 3;
  const folderApps = applications.filter((app) =>
    folder?.appIds.includes(app.id),
  );
  const cellWidth = gridWidth / columns;
  return (
    <Box
      testID="shell-launchpad"
      // A dialog named Launchpad, said by its title as it opens, as VoiceOver says Launchpad.
      accessibilityRole="dialog"
      accessibilityLabel="Launchpad"
      className="absolute inset-0 items-center overflow-hidden"
      style={{ zIndex: 20, display: open ? "flex" : "none" }}
    >
      {/* Outside the icons: a press closes it. Behind an open folder, nothing of the grid is read
          or reached (the folder is modal for assistive technologies). */}
      <Box
        className="absolute inset-0 items-center"
        importantForAccessibility={folder ? "no-hide-descendants" : "auto"}
        accessibilityElementsHidden={!!folder}
      >
      <PressArea
        accessibilityRole="button"
        accessibilityLabel={words.close}
        className="absolute inset-0 h-full w-full rounded-none border-0 bg-transparent p-0"
        onPress={dismiss}
      />
      <Box
        style={{
          position: "absolute",
          top: layout.searchTop,
          width: layout.searchWidth,
        }}
      >
        <BloomColorScope
          seed="#4464ba"
          mode="dark"
          tokens={SEARCH_TOKENS}
        >
          <Search
            ref={field}
            testID="launcher-search"
            label={messages.common.searchApplications}
            placeholder={words.search}
            accessibilityLabel={messages.common.searchApplications}
            iconSize={15}
            value={query}
            onChangeText={search}
            onClearText={() => search("")}
            fieldClassName="min-h-0 rounded-full"
            fieldChromeClassName="rounded-full border-white/25 bg-white/15"
            clearButtonProps={{ size: 20, glyphSize: 12 }}
            onKeyPress={(event) => {
              if (onKey(event.nativeEvent.key)) event.stopPropagation();
            }}
            onSubmitEditing={() => openEntry(entries[selected ?? 0])}
            style={{
              color: "#fff",
              fontSize: 13,
              height: SEARCH_HEIGHT,
              minHeight: SEARCH_HEIGHT,
              paddingTop: 0,
              paddingBottom: 0,
            }}
          />
        </BloomColorScope>
      </Box>
      {/* Presses between the icons go through to what closes it. */}
      <Box
        pointerEvents="box-none"
        className="flex-row flex-wrap"
        style={{ position: "absolute", top: layout.gridTop, width: gridWidth }}
      >
        {entries
          .slice(currentPage * capacity, (currentPage + 1) * capacity)
          .map((entry, index) => {
            const app = "app" in entry ? entry.app : undefined;
            const group = "folder" in entry ? entry.folder : undefined;
            const highlighted = selected === currentPage * capacity + index;
            return (
              <Box
                key={app?.id ?? group!.id}
                pointerEvents="box-none"
                className="items-center"
                style={{ width: cellWidth, height: rowHeight }}
              >
                <Button
                  ref={(node) => {
                    if (node) icons.current.set(keyOf(entry), node);
                    else icons.current.delete(keyOf(entry));
                  }}
                  testID={
                    app ? `launcher-${app.id}` : `launcher-folder-${group!.id}`
                  }
                  // Its name, as VoiceOver says a Launchpad icon; a folder says it is one.
                  accessibilityLabel={app ? app.name : words.folder(group!.name)}
                  pressed={highlighted}
                  appearance="plain"
                  className="flex-col items-center justify-start rounded-[18px] border-0 p-0"
                  style={{
                    width: Math.min(cellWidth, iconSize + 2 * HIGHLIGHT_PADDING + 24),
                    height: iconSize + 8 + labelSize + 4 + 2 * HIGHLIGHT_PADDING,
                    paddingTop: HIGHLIGHT_PADDING,
                    paddingBottom: HIGHLIGHT_PADDING,
                    // Launchpad's highlight: the icon and its label on a soft light plate.
                    backgroundColor: highlighted ? "#ffffff2e" : "transparent",
                  }}
                  onPress={() => openEntry(entry)}
                >
                  <Box className="items-center" style={{ gap: 8 }}>
                    {app ? (
                      <DraggableAppIcon app={app} size={iconSize} />
                    ) : (
                      <FolderIcon
                        apps={applications.filter((item) =>
                          group!.appIds.includes(item.id),
                        )}
                        size={iconSize}
                      />
                    )}
                    <Text
                      numberOfLines={1}
                      className="font-medium text-white"
                      style={[
                        LABEL_SHADOW,
                        {
                          fontSize: labelSize,
                          lineHeight: labelSize + 4,
                          maxWidth: cellWidth - 8,
                        },
                      ]}
                    >
                      {app?.name ?? group!.name}
                    </Text>
                  </Box>
                </Button>
              </Box>
            );
          })}
      </Box>
      {/* What the search found, a status the screen reader says politely as it changes, after
          what is typed is echoed, as VoiceOver says a search's results ("3 resultados"); nothing
          without a search. */}
      <Text role="status" style={SAID_ONLY}>
        {query ? messages.launchpad.results(entries.length) : ""}
      </Text>
      {!entries.length && (
        <Text
          className="text-center text-white/80"
          style={[
            LABEL_SHADOW,
            { position: "absolute", top: layout.gridTop + 40, fontSize: 15 },
          ]}
        >
          {words.noMatches(query)}
        </Text>
      )}
      {pageCount > 1 && (
        <Box
          className="absolute flex-row items-center justify-center"
          style={{ bottom: layout.dotsBottom, gap: 2 }}
        >
          {Array.from({ length: pageCount }, (_, index) => (
            <Button
              key={index}
              accessibilityLabel={words.page(index + 1)}
              pressed={index === currentPage}
              appearance="plain"
              className="h-6 w-5 rounded-full border-0 bg-transparent p-0"
              onPress={() => {
                setPage(index);
                setSelected(null);
              }}
            >
              <Box
                className="h-[7px] w-[7px] rounded-full"
                style={{
                  backgroundColor:
                    index === currentPage ? "#ffffff" : "#ffffff59",
                }}
              />
            </Button>
          ))}
        </Box>
      )}
      </Box>
      {folder && (
        <Box
          testID="launcher-folder"
          accessibilityViewIsModal
          className="absolute inset-0 items-center justify-center bg-black/20"
        >
          {/* Outside the folder: a press closes it, as Escape does; no stop of the keyboard's or
              the screen reader's, as a panel's backdrop is not. */}
          <PressArea
            accessible={false}
            className="absolute inset-0 h-full w-full rounded-none border-0 bg-transparent p-0"
            onPress={() => setFolder(null)}
          />
          <Box ref={folderPanel} style={{ width: folderWidth }}>
            <Text
              accessibilityRole="header"
              className="mb-5 text-center text-[24px] font-normal text-white"
              style={LABEL_SHADOW}
            >
              {folder.name}
            </Text>
            <Box
              className="flex-row flex-wrap rounded-[28px] border border-white/25 bg-white/20 p-6"
              style={{
                boxShadow: "0 18px 60px #00000030, inset 0 1px 0 #ffffff40",
              }}
            >
              {folderApps.map((app) => (
                <Button
                  key={app.id}
                  accessibilityLabel={app.name}
                  appearance="plain"
                  className="items-center justify-center rounded-[18px] border-0 bg-transparent p-0"
                  style={{
                    width: (folderWidth - 50) / folderColumns,
                    height: iconSize + labelSize + 40,
                  }}
                  onPress={() => launch(app.id)}
                >
                  <Box className="items-center" style={{ gap: 8 }}>
                    <DraggableAppIcon app={app} size={iconSize} />
                    <Text
                      numberOfLines={1}
                      className="font-medium text-white"
                      style={[LABEL_SHADOW, { fontSize: labelSize }]}
                    >
                      {app.name}
                    </Text>
                  </Box>
                </Button>
              ))}
            </Box>
          </Box>
        </Box>
      )}
    </Box>
  );
});

/** A folder's icon, as Launchpad's: a frosted tile showing its first nine applications. */
function FolderIcon({
  apps,
  size,
}: {
  apps: readonly ShellApplication[];
  size: number;
}) {
  // On the icon grid's body (apps.tsx): 87.5 % of the cell, Apple's corner.
  const body = size * 0.875;
  return (
    <Box
      style={{
        width: size,
        height: size,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Box
        className="flex-row flex-wrap content-start border border-white/25 bg-white/30"
        style={{
          width: body,
          height: body,
          borderRadius: body * 0.232,
          padding: body * 0.11,
          gap: body * 0.055,
        }}
      >
        {apps.slice(0, 9).map((item) => (
          <AppIcon key={item.id} app={item} size={body * 0.23} />
        ))}
      </Box>
    </Box>
  );
}
