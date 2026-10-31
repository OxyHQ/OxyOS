/**
 * Screenshots as a Mac takes them (docs/adr/0024-system-shortcuts.mdx, decision 10): the shell
 * draws the interface (Super+Shift+4's crosshair and its window picking, Super+Shift+5's
 * toolbar) and the thumbnail of a screenshot saved; the compositor captures, from its composed
 * frame, and the session writes the file. Rectangles here are logical pixels: an output's own
 * (its top-left corner at 0, 0), or the global space's, where outputs are placed side by side.
 */

/** A rectangle, logical pixels. */
export type Rect = { x: number; y: number; width: number; height: number };
export type Point = { x: number; y: number };

/** The display this shell runs on: where it is in the global space, and its scale. */
export type CaptureOutput = { id: number; x: number; y: number; scale: number };

/** What the person picks in the interface: a rectangle, or a window under the pointer. */
export type CaptureMode = "selection" | "window" | "toolbar";
/** The toolbar's three captures (screen recording is not OxyOS's yet). */
export type ToolbarMode = "screen" | "window" | "selection";
export type CaptureDestination = "desktop" | "documents";
/** The toolbar's timer, in seconds. */
export type CaptureTimer = 0 | 5 | 10;

/** The rectangle two corners span, its edges on whole logical pixels (a drag's press and the
    pointer now, either way round). */
export function spanned(a: Point, b: Point): Rect {
  const left = Math.round(Math.min(a.x, b.x));
  const top = Math.round(Math.min(a.y, b.y));
  const right = Math.round(Math.max(a.x, b.x));
  const bottom = Math.round(Math.max(a.y, b.y));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** `rect` kept within an output `width` × `height` (its own coordinates): none if nothing of it is. */
export function within(rect: Rect, width: number, height: number): Rect | null {
  const left = Math.max(0, rect.x);
  const top = Math.max(0, rect.y);
  const right = Math.min(width, rect.x + rect.width);
  const bottom = Math.min(height, rect.y + rect.height);
  if (right <= left || bottom <= top) return null;
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** An output's rectangle in the global space, from its own. */
export function toGlobal(rect: Rect, output: CaptureOutput): Rect {
  return { ...rect, x: rect.x + output.x, y: rect.y + output.y };
}

/** A global rectangle in an output's own coordinates. */
export function toLocal(rect: Rect, output: CaptureOutput): Rect {
  return { ...rect, x: rect.x - output.x, y: rect.y - output.y };
}

/**
 * How many of the display's own pixels a rectangle of the output (its own coordinates) is: each
 * edge at its scale, rounded to the nearest pixel, as the compositor places it and captures it
 * (1.25: 100 logical pixels from 0 are 125 pixels; from 1, 126 - 1).
 */
export function pixelSize(rect: Rect, scale: number): { width: number; height: number } {
  const at = (v: number) => Math.round(v * scale);
  return {
    width: at(rect.x + rect.width) - at(rect.x),
    height: at(rect.y + rect.height) - at(rect.y),
  };
}

/** What the crosshair shows beside a rectangle being dragged: its size in the display's pixels,
    as a Mac shows it. */
export function sizeLabel(rect: Rect, scale: number): string {
  const { width, height } = pixelSize(rect, scale);
  return `${width} × ${height}`;
}

/** The toolbar's selection before the person drags one: the middle half of the output. */
export function defaultSelection(width: number, height: number): Rect {
  return {
    x: Math.round(width / 4),
    y: Math.round(height / 4),
    width: Math.round(width / 2),
    height: Math.round(height / 2),
  };
}

/** The interface's state the compositor drives: its mode (Space switches the crosshair to picking
    a window and back), and how many times Return was pressed in the toolbar. */
export type CaptureInterface = {
  serial: number;
  mode: CaptureMode;
  output: number;
  confirm: number;
};

/** What a press in the interface picks: the crosshair's rectangle, or the window under it. */
export type CapturePick =
  | { kind: "area"; rect: Rect }
  | { kind: "window"; window: number }
  | { kind: "screens" };

/**
 * What the toolbar captures when the person presses Capturar (or Return): every display, the
 * window under the pointer (none when the pointer is over no window), or its selection (none
 * while it is empty), as a global pick.
 */
export function toolbarPick(
  mode: ToolbarMode,
  selection: Rect | null,
  target: number | null,
  output: CaptureOutput,
): CapturePick | null {
  switch (mode) {
    case "screen":
      return { kind: "screens" };
    case "window":
      return target ? { kind: "window", window: target } : null;
    case "selection":
      return selection && selection.width > 0 && selection.height > 0
        ? { kind: "area", rect: toGlobal(selection, output) }
        : null;
  }
}

/** The largest size of a thumbnail of `width` × `height` within `box` × `box`, proportions kept. */
export function thumbnailSize(width: number, height: number, box = 180): { width: number; height: number } {
  if (width <= 0 || height <= 0) return { width: box, height: Math.round(box * 0.625) };
  const ratio = Math.min(box / width, box / height);
  return { width: Math.max(1, Math.round(width * ratio)), height: Math.max(1, Math.round(height * ratio)) };
}

/** A thumbnail moved this far right (logical pixels) goes when it is let go, as a Mac's swiped
    away does; a press that moved less than `CLICK_SLOP` is a click, which opens it. */
export const DISMISS_DISTANCE = 60;
export const CLICK_SLOP = 4;

/** What letting a thumbnail go after moving it by (dx, dy) does. */
export function thumbnailRelease(dx: number, dy: number): "open" | "dismiss" | "stay" {
  if (Math.hypot(dx, dy) < CLICK_SLOP) return "open";
  if (dx >= DISMISS_DISTANCE) return "dismiss";
  return "stay";
}

/** How long a thumbnail shows, as a Mac's floating thumbnail. */
export const THUMBNAIL_MS = 5000;

/** A file's path as the `file://` URL an image is read from. */
export function fileUrl(path: string): string {
  return `file://${encodeURI(path)}`;
}

/** What a screen reader says once a screenshot is saved. */
export const SAVED_ANNOUNCEMENT = "Captura de pantalla guardada";

/** The seconds a timer still has, from its start and now (milliseconds), at least 1 while it runs. */
export function secondsLeft(timer: CaptureTimer, startedAt: number, now: number): number {
  return Math.max(0, Math.ceil((startedAt + timer * 1000 - now) / 1000));
}
