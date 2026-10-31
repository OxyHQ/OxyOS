import React, { useEffect, useRef, useState } from "react";
import {
  AccessibilityInfo,
  Animated,
  Image,
  StyleSheet,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type ViewStyle,
} from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import type { ShellScreenshot, ShellScreenshots } from "../model";
import type { ShellPlatform } from "../platform";
import {
  defaultSelection,
  fileUrl,
  SAVED_ANNOUNCEMENT,
  secondsLeft,
  sizeLabel,
  spanned,
  THUMBNAIL_MS,
  thumbnailRelease,
  thumbnailSize,
  toGlobal,
  toLocal,
  toolbarPick,
  within,
  type CaptureDestination,
  type CapturePick,
  type CaptureTimer,
  type Point,
  type Rect,
  type ToolbarMode,
} from "../screenshots";

/** The tint over the window a press captures, as a Mac's window picking shows it. */
const WINDOW_TINT = "#3d8bff59";
/** The selection being dragged: a light veil inside a thin white edge with a dark rim. */
const SELECTION_FILL = "#ffffff26";
const INK = "#1d2430";
/** The crosshair over the display while a rectangle is picked: CSS's cursor, which React Native's
    types do not list but its renderer, and OxyOS's, take (the compositor's Crosshair shape). */
const CROSSHAIR = "crosshair" as ViewStyle["cursor"];

const MODES: { mode: ToolbarMode; label: string; short: string }[] = [
  { mode: "screen", label: "Capturar pantalla completa", short: "Pantalla" },
  { mode: "window", label: "Capturar ventana seleccionada", short: "Ventana" },
  { mode: "selection", label: "Capturar parte seleccionada", short: "Selección" },
];
const DESTINATIONS: { destination: CaptureDestination; label: string }[] = [
  { destination: "desktop", label: "Escritorio" },
  { destination: "documents", label: "Documentos" },
];
const TIMERS: { timer: CaptureTimer; label: string }[] = [
  { timer: 0, label: "Ninguno" },
  { timer: 5, label: "5 segundos" },
  { timer: 10, label: "10 segundos" },
];

/**
 * Screenshots on this display, as a Mac takes them (docs/adr/0024-system-shortcuts.mdx, decision
 * 10). The compositor opens the interface and drives its keys; this shows it, in a surface of its
 * own that no screenshot captures:
 * - Super+Shift+4: the crosshair, on every display; a drag captures its rectangle. Space (the
 *   compositor's) switches to picking a window: the one under the pointer is tinted, a press
 *   captures it with its shadow.
 * - Super+Shift+5: the toolbar, on the display it was pressed for: the whole screen, a window or a
 *   selection; where to save and a timer; Capturar (or Return) captures, its close ends it.
 * A screenshot saved shows its thumbnail at the bottom-right corner for a few seconds: a press
 * opens it, dragging it right sends it away, and the screen reader says it was saved. The
 * interface never takes the keyboard: its keys are the compositor's.
 */
export function Screenshots({
  screenshots,
  platform,
  dock,
  dockHeight,
}: {
  screenshots: ShellScreenshots;
  platform: ShellPlatform;
  dock: boolean;
  dockHeight: number;
}) {
  const { Surface } = platform;
  const { active, output, target } = screenshots;
  // What the toolbar keeps while the session runs: its capture, where it saves and its timer.
  const [mode, setMode] = useState<ToolbarMode>("selection");
  const [destination, setDestination] = useState<CaptureDestination>("desktop");
  const [timer, setTimer] = useState<CaptureTimer>(0);
  const [options, setOptions] = useState(false);
  // The display's size (its own logical pixels), the selection and the drag making one.
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [selection, setSelection] = useState<Rect | null>(null);
  const [drag, setDrag] = useState<{ from: Point; to: Point } | null>(null);
  // A capture waiting for its timer, and when it began.
  const [countdown, setCountdown] = useState<{ pick: CapturePick; startedAt: number } | null>(null);
  const [now, setNow] = useState(0);

  const serial = active?.serial ?? null;
  const toolbar = active?.mode === "toolbar";
  // A new interface begins without a drag; the toolbar's begins with a selection to adjust.
  useEffect(() => {
    setDrag(null);
    setOptions(false);
    setSelection(toolbar && size.width > 0 ? defaultSelection(size.width, size.height) : null);
    if (serial === null) setCountdown(null);
  }, [serial]);

  const take = (pick: CapturePick) => {
    if (pick.kind === "screens") screenshots.capture("screens", 0, null, 0, destination);
    else if (pick.kind === "window") screenshots.capture("window", 0, null, pick.window, destination);
    else screenshots.capture("area", output.id, pick.rect, 0, destination);
  };
  // The toolbar's captures wait for its timer; the crosshair's never do.
  const fromToolbar = (pick: CapturePick | null) => {
    if (!pick) return;
    if (timer === 0) {
      take(pick);
      return;
    }
    const startedAt = Date.now();
    setNow(startedAt);
    setCountdown({ pick, startedAt });
  };
  useEffect(() => {
    if (!countdown) return undefined;
    if (secondsLeft(timer, countdown.startedAt, now) === 0) {
      setCountdown(null);
      take(countdown.pick);
      return undefined;
    }
    const tick = setTimeout(() => setNow(Date.now()), 250);
    return () => clearTimeout(tick);
  }, [countdown, now]);

  // The window under the pointer, on this display, in its coordinates.
  const targetHere = target && output ? toLocal(target, output) : null;
  const picking = active?.mode === "window" || (toolbar && mode === "window");
  const selecting = active?.mode === "selection" || (toolbar && mode === "selection");
  const toolbarPickNow = () => toolbarPick(mode, selection, target?.window ?? null, output);
  // Return pressed in the toolbar: what Capturar would capture (the window under the pointer, when
  // picking one). Counted per interface: a new one begins with none pressed.
  const confirmed = useRef<{ serial: number | null; count: number }>({ serial: null, count: 0 });
  useEffect(() => {
    if (!active) return;
    const last = confirmed.current;
    const pressed = last.serial === active.serial ? active.confirm > last.count : active.confirm > 0;
    confirmed.current = { serial: active.serial, count: active.confirm };
    if (pressed && toolbar && !countdown) fromToolbar(toolbarPickNow());
  }, [active?.serial, active?.confirm]);

  const point = (event: GestureResponderEvent): Point => ({
    x: event.nativeEvent.pageX,
    y: event.nativeEvent.pageY,
  });
  const press = (event: GestureResponderEvent) => {
    if (selecting) {
      const at = point(event);
      setDrag({ from: at, to: at });
    }
  };
  const move = (event: GestureResponderEvent) => {
    if (drag) setDrag({ from: drag.from, to: point(event) });
  };
  const release = (event: GestureResponderEvent) => {
    if (picking) {
      if (!target?.window) return;
      if (toolbar) fromToolbar({ kind: "window", window: target.window });
      else take({ kind: "window", window: target.window });
      return;
    }
    if (toolbar && mode === "screen") {
      fromToolbar({ kind: "screens" });
      return;
    }
    if (!drag) return;
    const rect = within(spanned(drag.from, point(event)), size.width, size.height);
    setDrag(null);
    if (!rect) return;
    if (toolbar) setSelection(rect);
    else take({ kind: "area", rect: toGlobal(rect, output) });
  };

  // The interface: on every display for the crosshair, on its own for the toolbar.
  const shown = !!active && !countdown && (!toolbar || active.output === output.id);
  const dragged = drag ? within(spanned(drag.from, drag.to), size.width, size.height) : null;
  const outlined = dragged ?? (toolbar && mode === "selection" ? selection : null);
  const bottom = (dock ? dockHeight : 0) + 24;

  return (
    <>
      {shown && (
        <Surface kind="overlay" excludeFromCapture style={StyleSheet.absoluteFill}>
          <Box
            testID="screenshot-interface"
            accessibilityLabel={picking ? "Elegir una ventana" : "Seleccionar una parte de la pantalla"}
            style={[StyleSheet.absoluteFill, { backgroundColor: "transparent", cursor: selecting ? CROSSHAIR : "auto" }]}
            onLayout={(event: LayoutChangeEvent) => {
              const { width, height } = event.nativeEvent.layout;
              setSize({ width, height });
              if (toolbar && !selection) setSelection(defaultSelection(width, height));
            }}
            onStartShouldSetResponder={() => true}
            onResponderGrant={press}
            onResponderMove={move}
            onResponderRelease={release}
          >
            {picking && targetHere && (
              <Box
                testID="screenshot-window"
                pointerEvents="none"
                style={{
                  position: "absolute",
                  left: targetHere.x,
                  top: targetHere.y,
                  width: targetHere.width,
                  height: targetHere.height,
                  backgroundColor: WINDOW_TINT,
                }}
              />
            )}
            {selecting && outlined && (
              <>
                <Box
                  testID="screenshot-selection"
                  pointerEvents="none"
                  style={{
                    position: "absolute",
                    left: outlined.x,
                    top: outlined.y,
                    width: outlined.width,
                    height: outlined.height,
                    backgroundColor: SELECTION_FILL,
                    borderWidth: 1,
                    borderColor: "#ffffff",
                    boxShadow: "0 0 0 1px #00000059",
                  }}
                />
                {/* Its size in the display's pixels, beside its far corner, as a Mac shows it. */}
                <Box
                  pointerEvents="none"
                  style={{
                    position: "absolute",
                    left: Math.min(outlined.x + outlined.width + 6, Math.max(0, size.width - 96)),
                    top: Math.min(outlined.y + outlined.height + 6, Math.max(0, size.height - 22)),
                    paddingHorizontal: 6,
                    paddingVertical: 2,
                    borderRadius: 4,
                    backgroundColor: "#000000a6",
                  }}
                >
                  <Text testID="screenshot-size" style={{ fontSize: 11, color: "#ffffff" }}>
                    {sizeLabel(outlined, output.scale)}
                  </Text>
                </Box>
              </>
            )}
            {toolbar && (
              <Box
                pointerEvents="box-none"
                style={{ position: "absolute", left: 0, right: 0, bottom, alignItems: "center", gap: 8 }}
              >
                {options && (
                  <CaptureOptions
                    destination={destination}
                    timer={timer}
                    setDestination={setDestination}
                    setTimer={setTimer}
                  />
                )}
                <CaptureToolbar
                  mode={mode}
                  setMode={(next) => {
                    setMode(next);
                    if (next === "selection" && !selection && size.width > 0) {
                      setSelection(defaultSelection(size.width, size.height));
                    }
                  }}
                  options={options}
                  toggleOptions={() => setOptions((open) => !open)}
                  // Picking a window, a press on it captures it: Capturar is for the others.
                  canCapture={mode !== "window" && !!toolbarPickNow()}
                  capture={() => fromToolbar(toolbarPickNow())}
                  close={screenshots.end}
                />
              </Box>
            )}
          </Box>
        </Surface>
      )}
      {/* The timer's seconds, over everything, taking no input: what is under it can still be
          used until it captures. Never captured. */}
      {countdown && (
        <Surface kind="notification" excludeFromCapture style={StyleSheet.absoluteFill}>
          <Box pointerEvents="none" style={[StyleSheet.absoluteFill, { alignItems: "center", justifyContent: "center" }]}>
            <Box
              testID="screenshot-countdown"
              style={{
                width: 96,
                height: 96,
                borderRadius: 48,
                alignItems: "center",
                justifyContent: "center",
                backgroundColor: "#1d2430b3",
              }}
            >
              <Text style={{ fontSize: 44, fontWeight: "600", color: "#ffffff" }}>
                {String(Math.max(1, secondsLeft(timer, countdown.startedAt, now)))}
              </Text>
            </Box>
          </Box>
        </Surface>
      )}
      <Thumbnail
        screenshots={screenshots}
        platform={platform}
        bottom={(dock ? dockHeight : 0) + 16}
        hidden={!!active}
      />
    </>
  );
}

/** The toolbar's row: the three captures, its options, Capturar and its close. */
function CaptureToolbar({
  mode,
  setMode,
  options,
  toggleOptions,
  canCapture,
  capture,
  close,
}: {
  mode: ToolbarMode;
  setMode: (mode: ToolbarMode) => void;
  options: boolean;
  toggleOptions: () => void;
  canCapture: boolean;
  capture: () => void;
  close: () => void;
}) {
  return (
    <Box
      testID="screenshot-toolbar"
      accessibilityRole="toolbar"
      accessibilityLabel="Captura de pantalla"
      // A press on it, between its buttons, is its own: never a capture of the screen beneath.
      onStartShouldSetResponder={() => true}
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 4,
        padding: 6,
        borderRadius: 14,
        backgroundColor: "#f5f7fbf2",
        boxShadow: "0 12px 40px #0b183a40, 0 0 0 .5px #0b183a1f",
        cursor: "auto",
      }}
    >
      <Button
        testID="screenshot-close"
        accessibilityLabel="Cerrar"
        appearance="plain"
        onPress={close}
        className="rounded-[8px] border-0 px-2"
        colors={{ background: "transparent", foreground: INK }}
      >
        ✕
      </Button>
      <Box style={{ width: 1, height: 24, backgroundColor: "#0b183a1f", marginHorizontal: 2 }} />
      {MODES.map((entry) => (
        <Button
          key={entry.mode}
          testID={`screenshot-mode-${entry.mode}`}
          accessibilityLabel={entry.label}
          appearance="plain"
          pressed={mode === entry.mode}
          onPress={() => setMode(entry.mode)}
          className="rounded-[8px] border-0 px-3"
          colors={{ background: mode === entry.mode ? "#0b183a1f" : "transparent", foreground: INK }}
        >
          {entry.short}
        </Button>
      ))}
      <Box style={{ width: 1, height: 24, backgroundColor: "#0b183a1f", marginHorizontal: 2 }} />
      <Button
        testID="screenshot-options"
        accessibilityLabel="Opciones"
        appearance="plain"
        pressed={options}
        onPress={toggleOptions}
        className="rounded-[8px] border-0 px-3"
        colors={{ background: options ? "#0b183a1f" : "transparent", foreground: INK }}
      >
        Opciones
      </Button>
      <Button
        testID="screenshot-capture"
        appearance="solid"
        tone="accent"
        disabled={!canCapture}
        onPress={capture}
        className="rounded-[8px] px-4"
      >
        Capturar
      </Button>
    </Box>
  );
}

/** The toolbar's options: where screenshots are saved, and the timer. */
function CaptureOptions({
  destination,
  timer,
  setDestination,
  setTimer,
}: {
  destination: CaptureDestination;
  timer: CaptureTimer;
  setDestination: (destination: CaptureDestination) => void;
  setTimer: (timer: CaptureTimer) => void;
}) {
  const row = (label: string, chosen: boolean, choose: () => void, testID: string) => (
    <Button
      key={testID}
      testID={testID}
      accessibilityLabel={label}
      appearance="plain"
      pressed={chosen}
      onPress={choose}
      className="justify-start rounded-[6px] border-0 px-2"
      colors={{ background: "transparent", foreground: INK }}
    >
      {`${chosen ? "✓ " : "    "}${label}`}
    </Button>
  );
  return (
    <Box
      testID="screenshot-options-menu"
      accessibilityRole="menu"
      accessibilityLabel="Opciones"
      onStartShouldSetResponder={() => true}
      style={{
        minWidth: 220,
        padding: 8,
        borderRadius: 12,
        backgroundColor: "#f5f7fbf2",
        boxShadow: "0 12px 40px #0b183a40, 0 0 0 .5px #0b183a1f",
        cursor: "auto",
      }}
    >
      <Text style={{ fontSize: 11, fontWeight: "600", color: "#5b6472", marginLeft: 8 }}>Guardar en</Text>
      <Box accessibilityRole="radiogroup" accessibilityLabel="Guardar en">
        {DESTINATIONS.map((entry) =>
          row(entry.label, destination === entry.destination, () => setDestination(entry.destination), `screenshot-save-${entry.destination}`),
        )}
      </Box>
      <Text style={{ fontSize: 11, fontWeight: "600", color: "#5b6472", marginLeft: 8, marginTop: 6 }}>
        Temporizador
      </Text>
      <Box accessibilityRole="radiogroup" accessibilityLabel="Temporizador">
        {TIMERS.map((entry) =>
          row(entry.label, timer === entry.timer, () => setTimer(entry.timer), `screenshot-timer-${entry.timer}`),
        )}
      </Box>
    </Box>
  );
}

/**
 * The latest screenshot of this display, floating at its bottom-right corner for a few seconds as
 * a Mac's: it slides in, a press opens it, dragging it moves it and dragging it right sends it
 * away (there is no dragging it into an application: OxyOS has no drag and drop between clients
 * yet). Said to the screen reader as it arrives; never captured.
 */
function Thumbnail({
  screenshots,
  platform,
  bottom,
  hidden,
}: {
  screenshots: ShellScreenshots;
  platform: ShellPlatform;
  bottom: number;
  hidden: boolean;
}) {
  const { Surface } = platform;
  const latest = screenshots.saved[screenshots.saved.length - 1];
  const mine = latest && latest.output === screenshots.output.id ? latest : undefined;
  const [shown, setShown] = useState<ShellScreenshot | null>(null);
  const seen = useRef<number | null>(null);
  useEffect(() => {
    if (!mine || mine.id === seen.current) return;
    seen.current = mine.id;
    setShown(mine);
    if (platform.announce) platform.announce(SAVED_ANNOUNCEMENT, true);
    else AccessibilityInfo.announceForAccessibility(SAVED_ANNOUNCEMENT);
  }, [mine?.id]);
  // A few seconds, then it goes; not while it is held.
  const [held, setHeld] = useState(false);
  useEffect(() => {
    if (!shown || held) return undefined;
    const gone = setTimeout(() => setShown(null), THUMBNAIL_MS);
    return () => clearTimeout(gone);
  }, [shown, held]);
  // Sliding in from the right edge, and following the pointer while it is dragged.
  const slide = useRef(new Animated.Value(0)).current;
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const from = useRef<Point | null>(null);
  const box = shown ? thumbnailSize(shown.width, shown.height) : { width: 0, height: 0 };
  useEffect(() => {
    if (!shown) return;
    setOffset({ x: 0, y: 0 });
    slide.setValue(box.width + 32);
    Animated.timing(slide, { toValue: 0, duration: 240, useNativeDriver: true }).start();
  }, [shown?.id]);
  if (!shown || hidden) return null;
  const at = (event: GestureResponderEvent): Point => ({ x: event.nativeEvent.pageX, y: event.nativeEvent.pageY });
  return (
    <Surface
      key={shown.id}
      kind="overlay"
      excludeFromCapture
      style={{ position: "absolute", right: 16, bottom, width: box.width, height: box.height }}
    >
      <Animated.View
        testID="screenshot-thumbnail"
        accessible
        accessibilityRole="button"
        accessibilityLabel="Captura de pantalla"
        accessibilityHint="Ábrela"
        onStartShouldSetResponder={() => true}
        onResponderGrant={(event: GestureResponderEvent) => {
          from.current = at(event);
          setHeld(true);
        }}
        onResponderMove={(event: GestureResponderEvent) => {
          if (!from.current) return;
          const now = at(event);
          setOffset({ x: now.x - from.current.x, y: now.y - from.current.y });
        }}
        onResponderRelease={(event: GestureResponderEvent) => {
          const start = from.current;
          from.current = null;
          setHeld(false);
          if (!start) return;
          const end = at(event);
          const outcome = thumbnailRelease(end.x - start.x, end.y - start.y);
          if (outcome === "open") {
            screenshots.open(shown.id);
            setShown(null);
          } else if (outcome === "dismiss") {
            setShown(null);
          } else {
            setOffset({ x: 0, y: 0 });
          }
        }}
        style={{
          width: box.width,
          height: box.height,
          borderRadius: 6,
          overflow: "hidden",
          borderWidth: 2,
          borderColor: "#ffffff",
          backgroundColor: "#ffffff",
          boxShadow: "0 8px 28px #0b183a59",
          transform: [{ translateX: Animated.add(slide, offset.x) }, { translateY: offset.y }],
        }}
      >
        <Image
          source={{ uri: fileUrl(shown.path) }}
          style={{ width: box.width - 4, height: box.height - 4 }}
          resizeMode="cover"
        />
      </Animated.View>
    </Surface>
  );
}
