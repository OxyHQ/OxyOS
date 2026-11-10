import React, { memo, useCallback, useEffect, useMemo, useRef, useState, type ComponentRef } from "react";
import {
  Image,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
  View,
  useWindowDimensions,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  type PointerEvent,
  type TextStyle,
} from "react-native";
import { Text } from "@oxy.so/bloom/typography";
import { BloomColorScope, useTheme } from "@oxy.so/bloom/theme";
import { usePlatformDrag } from "@oxy.so/bloom/platform-surface";
import { RiCalendarLine } from "@oxy.so/bloom/icons/RiCalendarLine";
import { RiChat3Line } from "@oxy.so/bloom/icons/RiChat3Line";
import { RiContactsBookLine } from "@oxy.so/bloom/icons/RiContactsBookLine";
import { RiFileCodeLine } from "@oxy.so/bloom/icons/RiFileCodeLine";
import { RiFileExcel2Line } from "@oxy.so/bloom/icons/RiFileExcel2Line";
import { RiFileImageLine } from "@oxy.so/bloom/icons/RiFileImageLine";
import { RiFileLine } from "@oxy.so/bloom/icons/RiFileLine";
import { RiFileMusicLine } from "@oxy.so/bloom/icons/RiFileMusicLine";
import { RiFilePdf2Line } from "@oxy.so/bloom/icons/RiFilePdf2Line";
import { RiFileTextLine } from "@oxy.so/bloom/icons/RiFileTextLine";
import { RiFileZipLine } from "@oxy.so/bloom/icons/RiFileZipLine";
import { RiFilmLine } from "@oxy.so/bloom/icons/RiFilmLine";
import { RiMailLine } from "@oxy.so/bloom/icons/RiMailLine";
import { RiSearchLine } from "@oxy.so/bloom/icons/RiSearchLine";
import { RiSettings3Line } from "@oxy.so/bloom/icons/RiSettings3Line";
import { RiSlideshow3Line } from "@oxy.so/bloom/icons/RiSlideshow3Line";
import { DRAG_SLOP } from "../app-drag";
import { AppIcon } from "../apps";
import { focusForAccessibility } from "../primitives";
import { filesUrl, opensType, parentOf } from "../desktop";
import type { ShellActions, ShellApplication } from "../model";
import type { SearchCategory, SearchItem, SearchResults, ShellKeyEvent, ShellPlatform } from "../platform";
import { size as sizeText } from "../sheet";
import {
  FIELD_HEIGHT,
  HEADER_HEIGHT,
  LIST_PADDING,
  PANEL_RADIUS,
  QUERY_LIMIT,
  ROW_HEIGHT,
  TOP_HIT_HEIGHT,
  draggable,
  kindName,
  locationText,
  localResults,
  moveSelection,
  openingOf,
  revealUrl,
  rowDetail,
  rowLabel,
  rowPlaces,
  rowsOf,
  scrolledTo,
  sectionsOf,
  sectionTitle,
  selectedAfter,
  spotlightLayout,
  type SpotlightMove,
  type SpotlightSection,
} from "../spotlight";
import type { ShellMessages } from "../messages";
import { useShellMessages } from "../useShellMessages";
import { useAccent } from "./accent";
import { DocumentIcon } from "./DocumentIcon";
import { FOLDER_ICON } from "./folder-icon";

const RULES = { filesUrl, opensType, parentOf };
const DRAG_ACTIONS = ["copy", "move", "link"] as const;

/*
 * The panel's look, at scale 1 (every box a multiple of 4, whole pixels at 1.25 too). The material
 * is opaque, light or dark, with a hairline ring and a soft shadow: a Mac's Spotlight blurs what is
 * behind it, which the shell cannot (it draws only its own surfaces), and a translucent panel
 * without a blur shows a window's text through its results. Its text in the system's greys, a row selected in the person's accent.
 */
const PALETTE = {
  light: {
    fill: "#f4f5f8",
    ring: "0 0 0 .5px #0b183a26",
    shadow: "0 24px 72px #0b183a40",
    text: "#1d1d1f",
    secondary: "#6e6e73",
    header: "#86868b",
    separator: "#0b183a1f",
    snippet: "#0b183a0d",
    placeholder: "#86868b",
  },
  dark: {
    fill: "#26272c",
    ring: "0 0 0 .5px #00000099, inset 0 .5px 0 #ffffff26",
    shadow: "0 24px 72px #00000073",
    text: "#f5f5f7",
    secondary: "#a1a1a6",
    header: "#8e8e93",
    separator: "#ffffff1f",
    snippet: "#ffffff0f",
    placeholder: "#8e8e93",
  },
} as const;
type Palette = (typeof PALETTE)["light"] | (typeof PALETTE)["dark"];

/** The field's text and its magnifier. */
const FIELD_TEXT = { fontSize: 22, lineHeight: 28 } as const;
const MAGNIFIER = 20;
/** A row's icon, the Top Hit's, and a row's text. */
const ROW_ICON = 20;
const TOP_HIT_ICON = 32;
const ROW_TEXT = { fontSize: 13, lineHeight: 16 } as const;
/** How much faster a row's detail shrinks than its name: the detail gives way first. */
const DETAIL_SHRINK = 100;
const ROW_INSET = 8;
const ROW_RADIUS = 8;
/** The preview: its padding, its icon, and the largest picture of an image. */
const PREVIEW_PADDING = 24;
const PREVIEW_ICON = 128;
const PICTURE_HEIGHT = 192;
const FACT_LABEL_WIDTH = 96;
const FACT_TEXT = { fontSize: 12, lineHeight: 16 } as const;

/** Read by the screen reader, drawn nowhere. */
const SAID_ONLY = { position: "absolute", width: 1, height: 1, overflow: "hidden", opacity: 0 } as const;

/** Each kind's glyph on a document's page. */
const GLYPHS: Record<SearchCategory, typeof RiFileLine> = {
  application: RiFileLine,
  settings: RiSettings3Line,
  folder: RiFileLine,
  document: RiFileLine,
  pdf: RiFilePdf2Line,
  text: RiFileTextLine,
  source: RiFileCodeLine,
  presentation: RiSlideshow3Line,
  spreadsheet: RiFileExcel2Line,
  image: RiFileImageLine,
  movie: RiFilmLine,
  music: RiFileMusicLine,
  archive: RiFileZipLine,
  contact: RiContactsBookLine,
  event: RiCalendarLine,
  mail: RiMailLine,
  message: RiChat3Line,
  other: RiFileLine,
};


/** A result's picture, `size` across: an application's icon (the result's application's, for its
    own items and a settings page), a folder's, an image's own, the result's icon, else its kind's
    page. */
const ResultIcon = memo(function ResultIcon({
  item,
  size,
  applications,
}: {
  item: SearchItem;
  size: number;
  applications: readonly ShellApplication[];
}) {
  const [failed, setFailed] = useState(false);
  const box = { width: size, height: size };
  const app = item.application ? applications.find((one) => one.id === item.application) : undefined;
  const ownPicture = item.category === "image" && item.uri && !failed;
  if (item.category === "folder") {
    return <Image source={FOLDER_ICON} style={box} resizeMode="contain" accessible={false} />;
  }
  if (ownPicture) {
    return (
      <Image source={{ uri: item.uri }} style={box} resizeMode="contain" accessible={false} onError={() => setFailed(true)} />
    );
  }
  if (app && (item.category === "application" || !item.uri)) return <AppIcon app={app} size={size} />;
  if (item.icon && !failed) {
    return (
      <Image source={{ uri: item.icon }} style={box} resizeMode="contain" accessible={false} onError={() => setFailed(true)} />
    );
  }
  return <DocumentIcon glyph={GLYPHS[item.category]} size={size} />;
});

/** The facts the preview lists, each a label and its value, those the result has. */
function factsOf(item: SearchItem, messages: ShellMessages): { label: string; value: string }[] {
  const words = messages.spotlight;
  const date = new Intl.DateTimeFormat(messages.locale, { dateStyle: "medium", timeStyle: "short" });
  const when = (time: number | undefined) => (time === undefined ? undefined : date.format(new Date(time)));
  const facts: [string, string | undefined][] = [
    [words.kind, kindName(item, messages)],
    [words.size, item.size === undefined || item.category === "folder" ? undefined : sizeText(item.size, messages)],
    [words.where, locationText(item, messages)],
    [words.created, when(item.created)],
    [words.modified, when(item.modified)],
    [words.lastOpened, when(item.lastUsed)],
    [words.tags, item.tags && item.tags.length > 0 ? item.tags.join(", ") : undefined],
  ];
  return facts.flatMap(([label, value]) => (value ? [{ label, value }] : []));
}

/** The selected result's preview, as a Mac's Spotlight shows it beside its list. */
const Preview = memo(function Preview({
  item,
  applications,
  palette,
  width,
}: {
  item: SearchItem;
  applications: readonly ShellApplication[];
  palette: Palette;
  width: number;
}) {
  const messages = useShellMessages();
  const picture = item.category === "image" && item.uri;
  const facts = factsOf(item, messages);
  return (
    <ScrollView
      testID="spotlight-preview"
      style={{ width }}
      contentContainerStyle={{ padding: PREVIEW_PADDING, alignItems: "center", gap: 16 }}
      // What it says is the row's, said as the row is reached: the reader does not stop here.
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
    >
      {picture ? (
        <PreviewPicture item={item} width={width - 2 * PREVIEW_PADDING} applications={applications} />
      ) : (
        <ResultIcon item={item} size={PREVIEW_ICON} applications={applications} />
      )}
      <Text numberOfLines={2} style={{ fontSize: 16, lineHeight: 20, fontWeight: "600", color: palette.text, textAlign: "center" }}>
        {item.title}
      </Text>
      {item.snippet ? (
        <View style={{ alignSelf: "stretch", backgroundColor: palette.snippet, borderRadius: 8, padding: 8 }}>
          <Text numberOfLines={6} style={{ ...FACT_TEXT, color: palette.secondary }}>
            {item.snippet}
          </Text>
        </View>
      ) : null}
      <View style={{ alignSelf: "stretch", gap: 4 }}>
        {facts.map((fact) => (
          <View key={fact.label} style={{ flexDirection: "row", gap: 8 }}>
            <Text numberOfLines={1} style={{ ...FACT_TEXT, width: FACT_LABEL_WIDTH, textAlign: "right", color: palette.secondary }}>
              {fact.label}
            </Text>
            <Text numberOfLines={2} style={{ ...FACT_TEXT, flex: 1, color: palette.text }}>
              {fact.value}
            </Text>
          </View>
        ))}
      </View>
    </ScrollView>
  );
});

/** An image's picture itself, fitted in the preview's width and its picture's height. */
function PreviewPicture({
  item,
  width,
  applications,
}: {
  item: SearchItem;
  width: number;
  applications: readonly ShellApplication[];
}) {
  const [failed, setFailed] = useState(false);
  if (failed) return <ResultIcon item={{ ...item, uri: undefined }} size={PREVIEW_ICON} applications={applications} />;
  return (
    <Image
      testID="spotlight-preview-picture"
      source={{ uri: item.uri }}
      style={{ width, height: PICTURE_HEIGHT, borderRadius: 4 }}
      resizeMode="contain"
      accessible={false}
      onError={() => setFailed(true)}
    />
  );
}

/**
 * A result's row: its icon, its title and, in grey, where it is or what it is; selected, on the
 * accent. A press opens it; the pointer over it selects it. A file's or a folder's row drags out
 * as the desktop's icons do (docs/adr/0047-finder-drags-and-the-desktop.mdx): a mouse press moved
 * past the slop is the platform's drag of the file, the row as it looks the drag's image.
 */
const Row = memo(function Row({
  item,
  top,
  selected,
  applications,
  palette,
  accent,
  onAccent,
  open,
  select,
  register,
}: {
  item: SearchItem;
  top: boolean;
  selected: boolean;
  applications: readonly ShellApplication[];
  palette: Palette;
  accent: string;
  onAccent: string;
  open: (item: SearchItem) => void;
  select: (id: string) => void;
  register: (id: string, view: ComponentRef<typeof View> | null) => void;
}) {
  const messages = useShellMessages();
  const platformDrag = usePlatformDrag();
  const view = useRef<ComponentRef<typeof View>>(null);
  const pressedAt = useRef<{ x: number; y: number } | null>(null);
  // A drag that started from the press: its release opens nothing.
  const dragged = useRef(false);
  const canDrag = !!platformDrag && draggable(item);
  const onPointerDown = useCallback((event: PointerEvent) => {
    const { button, ctrlKey, clientX, clientY } = event.nativeEvent;
    dragged.current = false;
    pressedAt.current = button === 0 && !ctrlKey ? { x: clientX, y: clientY } : null;
  }, []);
  const onPointerMove = useCallback(
    (event: PointerEvent) => {
      const at = pressedAt.current;
      if (!at || !platformDrag || !item.uri) return;
      const { clientX, clientY } = event.nativeEvent;
      if (Math.hypot(clientX - at.x, clientY - at.y) < DRAG_SLOP) return;
      pressedAt.current = null;
      dragged.current = true;
      platformDrag
        .start({ data: { files: [item.uri] }, actions: DRAG_ACTIONS, previewRef: view })
        .catch((error: unknown) => console.warn("Spotlight could not drag", { id: item.id, error }));
    },
    [platformDrag, item.uri, item.id],
  );
  const onPointerUp = useCallback(() => {
    pressedAt.current = null;
  }, []);
  const ink = selected ? onAccent : palette.text;
  const detail = rowDetail(item, messages, top);
  return (
    <Pressable
      ref={(node) => {
        view.current = node;
        register(item.id, node);
      }}
      collapsable={false}
      testID={`spotlight-row-${item.id}`}
      accessibilityRole="button"
      accessibilityLabel={rowLabel(item, messages)}
      accessibilityHint={canDrag ? `${messages.spotlight.openHint}. ${messages.spotlight.dragHint}` : messages.spotlight.openHint}
      accessibilityState={{ selected }}
      onPress={() => {
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        open(item);
      }}
      onHoverIn={() => select(item.id)}
      onPointerDownCapture={canDrag ? onPointerDown : undefined}
      onPointerMove={canDrag ? onPointerMove : undefined}
      onPointerUp={canDrag ? onPointerUp : undefined}
      onPointerCancel={canDrag ? onPointerUp : undefined}
      style={{
        height: top ? TOP_HIT_HEIGHT : ROW_HEIGHT,
        marginHorizontal: ROW_INSET,
        paddingHorizontal: 8,
        borderRadius: ROW_RADIUS,
        flexDirection: "row",
        alignItems: "center",
        gap: 8,
        backgroundColor: selected ? accent : "transparent",
      }}
    >
      <ResultIcon item={item} size={top ? TOP_HIT_ICON : ROW_ICON} applications={applications} />
      {/* The name first, as Spotlight's rows give it: what is beside it gives way before the name
          does (it shrinks a hundred times as fast, down to nothing), and the name is cut in the
          middle only when it alone is too wide, keeping its start and its extension. */}
      <Text numberOfLines={1} ellipsizeMode="middle" style={{ ...ROW_TEXT, flexShrink: 1, color: ink }}>
        {item.title}
      </Text>
      {detail ? (
        <Text
          numberOfLines={1}
          style={{ ...ROW_TEXT, flexShrink: DETAIL_SHRINK, minWidth: 0, color: selected ? onAccent : palette.secondary, opacity: selected ? 0.8 : 1 }}
        >
          {`— ${detail}`}
        </Text>
      ) : null}
    </Pressable>
  );
});

/** A section's header: its title, small and grey, a header for assistive technologies. */
function Header({ section, palette }: { section: SpotlightSection; palette: Palette }) {
  const messages = useShellMessages();
  return (
    <View style={{ height: HEADER_HEIGHT, justifyContent: "flex-end", paddingHorizontal: ROW_INSET + 8, paddingBottom: 4 }}>
      <Text accessibilityRole="header" numberOfLines={1} style={{ fontSize: 11, lineHeight: 16, fontWeight: "600", color: palette.header }}>
        {sectionTitle(section.key, messages)}
      </Text>
    </View>
  );
}

type Answer = { text: string; results: SearchResults };

/**
 * Spotlight (docs/adr/0051-search.mdx), as a Mac's: a floating panel a fifth of the way down the
 * screen, its large search field taking what is typed, and once something is found, the results
 * below it in sections (Top Hit first, then each kind where its best result ranks, a handful of
 * each) beside the selected one's preview. Typing searches the system's index, live as the index
 * changes; ↑ ↓ move through the results (Ctrl, the applications' ⌘, with them jumps a section);
 * Return opens the selected, Ctrl+Return or Super+Return shows it in Dropsea; Escape clears the
 * text, and closes once it is clear. The pointer over a row selects it, a press opens it, and a
 * file's row drags out.
 *
 * Its surface is the panel's region of an overlay: it floats above the windows and takes the
 * keyboard, and a press anywhere else gives the keyboard elsewhere, which closes it, the press
 * going where it was made, as a Mac's Spotlight closes. Without the system's search (the browser
 * preview), it finds the snapshot's applications by name.
 *
 * For a screen reader it is a dialog named Spotlight, its field first; each row is said by its
 * title, kind and place, the section headers are headers, the count of results is said politely as
 * it changes, and the row the arrows reach takes the reader's focus, as VoiceOver's cursor follows
 * Spotlight's selection; typing gives the field its focus back.
 */
export const Spotlight = memo(function Spotlight(props: {
  platform: ShellPlatform;
  applications: readonly ShellApplication[];
  actions: ShellActions;
  /** Launches or brings forward an application (the shell's own, as the dock's press). */
  launch: (id: string) => void;
  /** Closes it: `dismissed` when nothing was opened (the keyboard goes back to the window). */
  close: (dismissed: boolean) => void;
  /** Each new value puts the keyboard on its field again. */
  focusRequest: number;
  /** Each new value shows the selected result in Dropsea (Super+Return). */
  revealRequest: number;
  /** Each new value is Escape (the platform's dismiss). */
  escapeRequest: number;
  /** Whether it may have the keyboard: not under a system sheet. */
  keyboard: boolean;
}) {
  return (
    <BloomColorScope colorPreset={useAccent()} asChild>
      <View pointerEvents="box-none" style={StyleSheet.absoluteFill}>
        <SpotlightPanel {...props} />
      </View>
    </BloomColorScope>
  );
});

function SpotlightPanel({
  platform,
  applications,
  actions,
  launch,
  close,
  focusRequest,
  revealRequest,
  escapeRequest,
  keyboard,
}: Parameters<typeof Spotlight>[0]) {
  const { Surface, search } = platform;
  const messages = useShellMessages();
  const words = messages.spotlight;
  const theme = useTheme();
  const palette: Palette = theme.isDark ? PALETTE.dark : PALETTE.light;
  const { width: screenWidth, height: screenHeight } = useWindowDimensions();
  const layout = spotlightLayout(screenWidth, screenHeight);
  const [text, setText] = useState("");
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const field = useRef<ComponentRef<typeof TextInput>>(null);
  const list = useRef<ComponentRef<typeof ScrollView>>(null);
  const scrolled = useRef(0);
  const rows = useRef(new Map<string, ComponentRef<typeof View>>());
  // Whether the arrows moved the reader's focus off the field, to give it back on typing.
  const arrowed = useRef(false);

  // Each change of the text asks again; an answer to an earlier question is dropped. The index
  // changing while there is text asks again too. With each answer, the selection: Top Hit for a new
  // text, the same row for the same text (the index changed under it).
  const asked = useRef(0);
  const answered = useRef<string | null>(null);
  const latestApplications = useRef(applications);
  latestApplications.current = applications;
  const wanted = text.trim();
  useEffect(() => {
    const accept = (next: Answer) => {
      const same = next.text === answered.current;
      answered.current = next.text;
      setAnswer(next);
      setSelected((before) => selectedAfter(sectionsOf(next.results), before, same));
    };
    if (!wanted) {
      asked.current += 1;
      answered.current = null;
      setAnswer(null);
      setSelected(null);
      return undefined;
    }
    const ask = () => {
      const mine = ++asked.current;
      if (!search) {
        accept({ text: wanted, results: localResults(latestApplications.current, wanted) });
        return;
      }
      search
        .query({ text: wanted, limit: QUERY_LIMIT, language: messages.language })
        .then(
          (results) => {
            if (mine === asked.current) accept({ text: wanted, results });
          },
          (error: unknown) => {
            console.warn("Spotlight's search failed", { error });
            if (mine === asked.current) accept({ text: wanted, results: { items: [], topHit: null, indexing: false } });
          },
        );
    };
    ask();
    return search ? search.watch(ask) : undefined;
  }, [wanted, search, messages.language]);

  const sections = useMemo(() => (answer ? sectionsOf(answer.results) : []), [answer]);
  const shownRows = useMemo(() => rowsOf(sections), [sections]);
  const places = useMemo(() => rowPlaces(sections), [sections]);
  const selectedItem = shownRows.find((item) => item.id === selected) ?? null;

  // The results stay while the next text's are asked for, so typing never collapses the panel;
  // it closes to its field alone when there is no text, or nothing was found.
  const showResults = wanted.length > 0 && shownRows.length > 0;
  const height = FIELD_HEIGHT + (showResults ? layout.resultsHeight : 0);

  // Opening and revealing (docs/adr/0051-search.mdx).
  const open = useCallback(
    (item: SearchItem) => {
      const opening = openingOf(item, applications, RULES);
      if (!opening) return;
      if (opening.kind === "application") launch(opening.id);
      else if (opening.kind === "files") actions.openFiles?.(opening.application, opening.uris);
      else actions.openUrl?.(opening.url);
      search?.used(item.id);
      close(false);
    },
    [applications, actions, launch, search, close],
  );
  const reveal = useCallback(() => {
    const item = selectedItem;
    const url = item ? revealUrl(item, RULES) : null;
    if (!url) return;
    actions.openUrl?.(url);
    close(false);
  }, [selectedItem, actions, close]);
  const dismiss = useCallback(() => {
    if (text) change("");
    else close(true);
  }, [text, close]);

  // The selection moved from the keyboard: the row shows, and takes the reader's focus.
  const [readerRequest, setReaderRequest] = useState(0);
  const moveTo = (next: string | null) => {
    if (next === null) return;
    setSelected(next);
    arrowed.current = true;
    setReaderRequest((value) => value + 1);
    const place = places.rows.get(next);
    const first = sections.some((section) => section.items[0]?.id === next);
    if (place) {
      const offset = scrolledTo(place, scrolled.current, layout.resultsHeight, first);
      if (offset !== scrolled.current) list.current?.scrollTo({ y: offset, animated: false });
    }
  };
  useEffect(() => {
    if (readerRequest > 0 && selected) focusForAccessibility(rows.current.get(selected));
  }, [readerRequest]);

  const change = (value: string) => {
    if (arrowed.current) {
      arrowed.current = false;
      focusForAccessibility(field.current as unknown as ComponentRef<typeof View>);
    }
    setText(value);
    scrolled.current = 0;
    list.current?.scrollTo({ y: 0, animated: false });
  };

  // What a key does, from the platform's keys before the field, else the field's own.
  const onKey = (key: string, ctrl: boolean): boolean => {
    if (key === "Escape") {
      dismiss();
      return true;
    }
    if (key === "Enter") {
      if (ctrl) reveal();
      else if (selectedItem) open(selectedItem);
      return true;
    }
    if (key === "ArrowDown" || key === "ArrowUp") {
      const move: SpotlightMove =
        key === "ArrowDown" ? (ctrl ? "next-section" : "down") : ctrl ? "previous-section" : "up";
      moveTo(moveSelection(sections, selected, move));
      return true;
    }
    // Tab stays in the field: Spotlight has nothing else the keyboard goes to.
    return key === "Tab";
  };
  const latestKey = useRef(onKey);
  latestKey.current = onKey;
  useEffect(() => {
    if (!keyboard || !platform.keyboard) return undefined;
    return platform.keyboard.subscribe((event) =>
      event.altKey || event.metaKey || event.shiftKey ? false : latestKey.current(event.key, !!event.ctrlKey),
    );
  }, [keyboard, platform.keyboard]);

  const latestReveal = useRef(reveal);
  latestReveal.current = reveal;
  const latestEscape = useRef(dismiss);
  latestEscape.current = dismiss;
  const lastReveal = useRef(revealRequest);
  useEffect(() => {
    if (lastReveal.current !== revealRequest) latestReveal.current();
    lastReveal.current = revealRequest;
  }, [revealRequest]);
  const lastEscape = useRef(escapeRequest);
  useEffect(() => {
    if (lastEscape.current !== escapeRequest) latestEscape.current();
    lastEscape.current = escapeRequest;
  }, [escapeRequest]);
  // Opened, the field takes what is typed and the reader's focus; asked again, the keyboard.
  useEffect(() => {
    field.current?.focus();
    focusForAccessibility(field.current as unknown as ComponentRef<typeof View>);
  }, []);
  const lastFocus = useRef(focusRequest);
  useEffect(() => {
    if (lastFocus.current !== focusRequest) field.current?.focus();
    lastFocus.current = focusRequest;
  }, [focusRequest]);

  // The count found, said politely as it changes, after the field's echo, as VoiceOver says it.
  const count = wanted && answer?.text === wanted ? messages.launchpad.results(shownRows.length) : "";

  const register = useCallback((id: string, view: ComponentRef<typeof View> | null) => {
    if (view) rows.current.set(id, view);
    else rows.current.delete(id);
  }, []);
  const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    scrolled.current = event.nativeEvent.contentOffset.y;
  }, []);
  const area = useMemo(
    () => ({ position: "absolute", left: layout.left, top: layout.top, width: layout.width, height }) as const,
    [layout.left, layout.top, layout.width, height],
  );
  const accent = theme.colors.primary;
  const onAccent = theme.colors.primaryForeground;
  return (
    // Positioned here, so the region is where the panel is wherever the platform draws it.
    <View pointerEvents="box-none" style={area}>
      <Surface
        kind="overlay"
        style={StyleSheet.absoluteFill}
        keyboard={keyboard}
        onKeyboardLost={() => close(false)}
      >
        <View
          testID="shell-spotlight"
          role="dialog"
          accessibilityLabel={words.label}
          style={{
            width: layout.width,
            height,
            borderRadius: PANEL_RADIUS,
            backgroundColor: palette.fill,
            boxShadow: `${palette.shadow}, ${palette.ring}`,
          }}
        >
          <View style={{ flex: 1, borderRadius: PANEL_RADIUS, overflow: "hidden" }}>
            {/* The field is the panel's top, as a Mac's: a magnifier and large text, no box of its
                own. The text's own field, as the lock screen's password is. */}
            <View style={styles.field}>
              <RiSearchLine width={MAGNIFIER} height={MAGNIFIER} fill={palette.secondary} />
              <TextInput
                ref={field}
                testID="spotlight-search"
                accessibilityRole="search"
                accessibilityLabel={words.placeholder}
                placeholder={words.placeholder}
                placeholderTextColor={palette.placeholder}
                value={text}
                onChangeText={change}
                autoCorrect={false}
                autoCapitalize="none"
                autoComplete="off"
                spellCheck={false}
                returnKeyType="go"
                submitBehavior="submit"
                onKeyPress={(event) => {
                  const native = event.nativeEvent as ShellKeyEvent;
                  if (native.altKey || native.metaKey || native.shiftKey) return;
                  if (onKey(native.key, !!native.ctrlKey)) {
                    event.preventDefault();
                    event.stopPropagation();
                  }
                }}
                onSubmitEditing={() => {
                  if (selectedItem) open(selectedItem);
                }}
                style={[styles.input, WEB_OUTLINE, { color: palette.text }]}
              />
            </View>
            {showResults && (
              <View style={{ height: layout.resultsHeight, flexDirection: "row" }}>
                <View pointerEvents="none" style={[styles.rule, { top: 0, left: 0, right: 0, height: StyleSheet.hairlineWidth, backgroundColor: palette.separator }]} />
                <View style={{ width: layout.listWidth }}>
                <ScrollView
                  ref={list}
                  testID="spotlight-results"
                  accessibilityRole="list"
                  style={{ flex: 1 }}
                  contentContainerStyle={{ paddingVertical: LIST_PADDING }}
                  onScroll={onScroll}
                  scrollEventThrottle={16}
                >
                  {sections.map((section) => (
                    <View key={section.key}>
                      <Header section={section} palette={palette} />
                      {section.items.map((item) => (
                        <Row
                          key={item.id}
                          item={item}
                          top={section.key === "top-hit"}
                          selected={item.id === selected}
                          applications={applications}
                          palette={palette}
                          accent={accent}
                          onAccent={onAccent}
                          open={open}
                          select={setSelected}
                          register={register}
                        />
                      ))}
                    </View>
                  ))}
                  {answer?.results.indexing ? (
                    <Text style={{ ...FACT_TEXT, color: palette.secondary, paddingHorizontal: ROW_INSET + 8, paddingTop: 8 }}>
                      {words.indexing}
                    </Text>
                  ) : null}
                </ScrollView>
                </View>
                {layout.preview && selectedItem ? (
                  <View style={{ flex: 1, overflow: "hidden" }}>
                    <View pointerEvents="none" style={[styles.rule, { top: 0, bottom: 0, left: 0, width: StyleSheet.hairlineWidth, backgroundColor: palette.separator }]} />
                    <Preview item={selectedItem} applications={applications} palette={palette} width={layout.width - layout.listWidth} />
                  </View>
                ) : null}
              </View>
            )}
            <Text role="status" style={SAID_ONLY}>
              {count}
            </Text>
          </View>
        </View>
      </Surface>
    </View>
  );
}

const styles = StyleSheet.create({
  rule: { position: "absolute", zIndex: 1 },
  field: { height: FIELD_HEIGHT, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 20 },
  input: {
    flex: 1,
    height: FIELD_HEIGHT,
    padding: 0,
    borderWidth: 0,
    backgroundColor: "transparent",
    ...FIELD_TEXT,
  },
});
/** React Native for the web draws the browser's focus outline around the field's own box, which
    the panel, its field, does not have: as Bloom's fields, it is taken off there, and never
    applied elsewhere (these are web style properties, absent from React Native's types). */
const WEB_OUTLINE: TextStyle | undefined =
  Platform.OS === "web" ? ({ outlineWidth: 0, outlineStyle: "none" } as unknown as TextStyle) : undefined;
