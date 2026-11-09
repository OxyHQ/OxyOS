import type { SwipeFingers } from "./model";

/**
 * What Trackpad's More Gestures draw beside each row (docs/adr/0046-interactive-gestures.mdx): a
 * Mac's Trackpad settings plays a film of the hand for each choice, which Bloom cannot play, so
 * Settings draws the trackpad, the fingers on it and the way they go, and moves them as the
 * gesture moves when its choice changes or the pointer comes over its row. Kept apart from the
 * drawing so that what it shows at rest, and that a play comes to rest, can be checked.
 */
export type Gesture = "desktops" | "missionControl" | "appExpose" | "launchpad";

/** The trackpad drawn, in logical pixels. */
export const PAD = { width: 52, height: 36, radius: 8 } as const;
/** A finger's dot's radius. */
export const FINGER_RADIUS = 3;
/** One go of the gesture, and how many a play is: two, as a Mac's film plays it twice over, then
    it rests on its still and draws nothing more. */
export const CYCLE_MS = 1200;
export const CYCLES = 2;
export const PLAY_MS = CYCLE_MS * CYCLES;

/** A finger at rest, and how far the gesture takes it. */
export type Finger = { x: number; y: number; dx: number; dy: number };

/** How many fingers the drawing shows for a swipe's choice: off is drawn as three, dimmed. */
export function fingerCount(choice: SwipeFingers): 3 | 4 {
  return choice === "four" ? 4 : 3;
}

/** Fingers side by side across the middle of the pad, at `y`. */
function row(count: 3 | 4, y: number, dx: number, dy: number): Finger[] {
  const spacing = 8;
  const first = PAD.width / 2 - ((count - 1) * spacing) / 2;
  return Array.from({ length: count }, (_, index) => ({ x: first + index * spacing, y, dx, dy }));
}

/** Where Launchpad's pinch closes on: the thumb and the three fingers above it come together there,
    each going `PINCH` of the way. */
export const PINCH_CENTRE = { x: 26, y: 19 } as const;
const PINCH = 0.6;

/** The fingers of a gesture at rest and where it takes them: up for Mission Control, down for App
    Exposé, across for the desktops (left on the first go, right on the second), and Launchpad's
    thumb lower left with three fingers above it, drawn in. */
export function fingersOf(gesture: Gesture, count: 3 | 4): Finger[] {
  switch (gesture) {
    case "missionControl":
      return row(count, 25, 0, -12);
    case "appExpose":
      return row(count, 11, 0, 12);
    case "desktops":
      return row(count, 15, 10, 0);
    case "launchpad":
      return [
        { x: 14, y: 28 },
        { x: 20, y: 11 },
        { x: 28, y: 8 },
        { x: 36, y: 11 },
      ].map(({ x, y }) => ({ x, y, dx: (PINCH_CENTRE.x - x) * PINCH, dy: (PINCH_CENTRE.y - y) * PINCH }));
  }
}

/** The way the gesture goes, drawn still under the fingers as SVG paths: each finger's trail and a
    chevron where it heads; for the desktops an arrow both ways under them. */
export function marksOf(gesture: Gesture, fingers: readonly Finger[]): string[] {
  const trails = fingers.map(({ x, y, dx, dy }) => `M${x} ${y}L${x + dx} ${y + dy}`);
  switch (gesture) {
    case "missionControl":
      return [...trails, "M22 8L26 4L30 8"];
    case "appExpose":
      return [...trails, "M22 28L26 32L30 28"];
    case "desktops":
      return ["M12 28L40 28", "M16 25L12 28L16 31", "M36 25L40 28L36 31"];
    case "launchpad":
      return trails;
  }
}

/** The drawing at a moment of a play: how far along its way each finger is (0 to 1), how opaque,
    and which way across (the desktops' first go is to the left, the second to the right). */
export type GestureFrame = { along: number; opacity: number; direction: 1 | -1 };

/** At rest: the fingers where they start, fully drawn. */
export const REST_FRAME: GestureFrame = { along: 0, opacity: 1, direction: 1 };

/** Where a go starts once its fingers have come in, and where it has gone all the way. */
const ARRIVED = 0.15;
const GONE = 0.75;

/**
 * The frame `t` cycles into a play (0 to `CYCLES`), on the UI thread: each go the fingers move
 * along their way, easing in and out, then fade where they arrived and come in again where they
 * start. A play starts at the still and ends on it: the first go starts with its fingers already
 * there, and after the last they come in again and rest.
 */
export function frameAt(t: number): GestureFrame {
  "worklet";
  if (!(t > 0 && t < CYCLES)) return REST_FRAME;
  const shifted = t + ARRIVED;
  const cycle = Math.floor(shifted);
  const p = shifted - cycle;
  const x = Math.min(1, Math.max(0, (p - ARRIVED) / (GONE - ARRIVED)));
  const along = p < ARRIVED ? 0 : x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2;
  const opacity = p < ARRIVED ? p / ARRIVED : p < GONE ? 1 : 1 - (p - GONE) / (1 - GONE);
  return { along, opacity, direction: cycle % 2 === 0 ? -1 : 1 };
}
