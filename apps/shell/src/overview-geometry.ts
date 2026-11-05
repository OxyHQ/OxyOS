export type OverviewGeometry = {
  x: number;
  y: number;
  width: number;
  cardWidth: number;
  cardHeight: number;
  gap: number;
  inset: number;
  top: number;
  count: number;
};

/** Pointer coordinates are output-relative; gaps and shell panels are never drop targets. */
export function workspaceAtPoint(
  x: number,
  y: number,
  scroll: number,
  geometry: OverviewGeometry,
): number {
  "worklet";
  const localX = x - geometry.x;
  const localY = y - geometry.y;
  if (
    localX < 0 ||
    localX >= geometry.width ||
    localY < geometry.top ||
    localY >= geometry.top + geometry.cardHeight
  )
    return -1;
  const contentX = localX + scroll - geometry.inset;
  const stride = geometry.cardWidth + geometry.gap;
  const index = Math.floor(contentX / stride);
  if (
    index < 0 ||
    index >= geometry.count ||
    contentX - index * stride >= geometry.cardWidth
  )
    return -1;
  return index;
}

export function nearestWorkspace(
  offset: number,
  stride: number,
  count: number,
): number {
  return Math.max(0, Math.min(count - 1, Math.round(offset / stride)));
}

/**
 * Windows in rows, as the overview lays them out (docs/adr/0022-window-views.mdx) and App Exposé
 * does (docs/adr/0044-app-expose.mdx): `columns` windows a row, in order; each row's windows one
 * height, which keeps their proportions (`ratios`, width over height) and fits the row's width
 * with `gap` between them; the rows share `height` evenly, with `gap` between them, each row's
 * height clamped to [`minRow`, `maxRow`] and holding `chrome` beside its windows (a card's title
 * bar, a window's name under it). `contentHeight` is the windows' height in a row.
 */
export type WindowRow = { indices: number[]; contentHeight: number };

export function windowRows(
  ratios: readonly number[],
  area: {
    width: number;
    height: number;
    gap: number;
    columns: number;
    chrome: number;
    minRow?: number;
    maxRow?: number;
  },
): WindowRow[] {
  const columns = Math.max(1, Math.floor(area.columns));
  const count = Math.ceil(ratios.length / columns);
  const rowHeight = Math.max(
    area.minRow ?? 0,
    Math.min(
      area.maxRow ?? Infinity,
      (area.height - Math.max(0, count - 1) * area.gap) / Math.max(1, count),
    ),
  );
  return Array.from({ length: count }, (_, row) => {
    const indices = ratios
      .map((_, index) => index)
      .slice(row * columns, (row + 1) * columns);
    const ratioSum = indices.reduce((sum, index) => sum + ratios[index]!, 0);
    return {
      indices,
      contentHeight: Math.max(
        0,
        Math.min(
          rowHeight - area.chrome,
          (area.width - (indices.length - 1) * area.gap) / ratioSum,
        ),
      ),
    };
  });
}

export type Rect = { x: number; y: number; width: number; height: number };

/** A window as App Exposé lays it out: its proportions, and its own height, which it is never
    drawn taller than (a Mac's App Exposé never enlarges a window). */
export type ExposedWindow = { ratio: number; height?: number };

/**
 * Where App Exposé shows an application's windows (docs/adr/0044-app-expose.mdx): as large as
 * they fit in `area`, in the overview's rows (`windowRows`), with the number of columns that
 * gives them the most room, each no taller than it is; the rows centred in the area together and
 * each row centred across it, `label` kept under each window for its name. In the windows' order;
 * none overlaps another, and every one is inside the area.
 */
export function exposeLayout(
  windows: readonly ExposedWindow[],
  area: Rect,
  gap: number,
  label: number,
): Rect[] {
  if (windows.length === 0 || area.width <= 0 || area.height <= 0) return [];
  const ratios = windows.map((window) => window.ratio);
  const sizes = (rows: WindowRow[]) =>
    rows.map((row) =>
      row.indices.map((index) => {
        const own = windows[index]!.height;
        const height = Math.min(row.contentHeight, own && own > 0 ? own : Infinity);
        return { width: height * ratios[index]!, height };
      }),
    );
  // The columns that leave the windows the most area, the fewest rows on a tie.
  let best: { rows: WindowRow[]; area: number } | null = null;
  for (let columns = 1; columns <= windows.length; columns++) {
    const rows = windowRows(ratios, {
      width: area.width,
      height: area.height,
      gap,
      columns,
      chrome: label,
    });
    const room = sizes(rows)
      .flat()
      .reduce((sum, size) => sum + size.width * size.height, 0);
    if (!best || room > best.area + 1e-6) best = { rows, area: room };
  }
  const rows = best!.rows;
  const sized = sizes(rows);
  const heights = sized.map(
    (row) => Math.max(...row.map((size) => size.height)) + label,
  );
  const block =
    heights.reduce((sum, height) => sum + height, 0) +
    Math.max(0, rows.length - 1) * gap;
  const placed: Rect[] = new Array(windows.length);
  let y = area.y + Math.max(0, (area.height - block) / 2);
  rows.forEach((row, r) => {
    const widths = sized[r]!.map((size) => size.width);
    const span =
      widths.reduce((sum, width) => sum + width, 0) +
      Math.max(0, widths.length - 1) * gap;
    let x = area.x + Math.max(0, (area.width - span) / 2);
    const tallest = heights[r]! - label;
    row.indices.forEach((index, i) => {
      const size = sized[r]![i]!;
      // Each window centred in its row's height, its name under it.
      placed[index] = {
        x: Math.round(x),
        y: Math.round(y + (tallest - size.height) / 2),
        width: Math.floor(size.width),
        height: Math.floor(size.height),
      };
      x += size.width + gap;
    });
    y += heights[r]! + gap;
  });
  return placed;
}

/**
 * The window the arrows move to from `from` among `rects` (App Exposé's keys, as a Mac's): ← and
 * → the nearest whose middle is that way in its row (sharing some of its height), else the one
 * before or after it in order; ↑ and ↓ the nearest whose middle is above or below, by its
 * middle's distance across. `from` itself when there is none that way.
 */
export function neighbour(
  rects: readonly Rect[],
  from: number,
  direction: "left" | "right" | "up" | "down",
): number {
  const at = rects[from];
  if (!at) return from;
  const middle = (rect: Rect) => ({
    x: rect.x + rect.width / 2,
    y: rect.y + rect.height / 2,
  });
  const here = middle(at);
  const horizontal = direction === "left" || direction === "right";
  const sign = direction === "left" || direction === "up" ? -1 : 1;
  let best = from;
  let distance = Infinity;
  rects.forEach((rect, index) => {
    if (index === from) return;
    const there = middle(rect);
    const along = horizontal ? there.x - here.x : there.y - here.y;
    if (along * sign <= 0) return;
    if (horizontal) {
      const shares =
        rect.y < at.y + at.height && at.y < rect.y + rect.height;
      if (!shares) return;
    }
    const across = horizontal ? there.y - here.y : there.x - here.x;
    const score = Math.abs(along) + 2 * Math.abs(across);
    if (score < distance) {
      distance = score;
      best = index;
    }
  });
  if (best === from && horizontal) {
    const next = from + sign;
    if (next >= 0 && next < rects.length) return next;
  }
  return best;
}
