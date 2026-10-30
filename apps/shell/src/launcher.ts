/**
 * Launchpad's layout and keyboard (docs/adr/0009-session-shell.mdx), as a Mac's: a page of seven
 * columns and five rows of icons spread over the screen below a search field, an icon a little
 * over half its column, the pages' dots at the bottom; the arrows move a highlight through the
 * icons, across pages, and Return opens the one highlighted. Pure, so it is tested without a
 * renderer.
 */

/** Where the page's parts go, in logical pixels, for a launcher `width` × `height`. */
export type LauncherLayout = {
  columns: number;
  rows: number;
  /** The grid's width; it is centred. */
  gridWidth: number;
  /** Where the grid's first row begins, from the launcher's top. */
  gridTop: number;
  rowHeight: number;
  iconSize: number;
  /** The labels' size. */
  labelSize: number;
  /** The search field's top and width. */
  searchTop: number;
  searchWidth: number;
  /** The page dots' distance from the bottom. */
  dotsBottom: number;
};

/** The search field's height (a macOS 26 capsule). */
export const SEARCH_HEIGHT = 32;
/** An icon's highlight's padding around it and its label. */
export const HIGHLIGHT_PADDING = 8;

/** The smallest icon. */
const LEAST_ICON = 48;

/**
 * A page of a launcher `width` × `height`: seven columns and five rows on a computer's screen
 * (four columns on a narrow one, fewer rows on a short one), the grid 78 % of the width (at most
 * 1160) and the rows spread between the search field and the dots; an icon 56 % of its column,
 * never so tall its label leaves its row, between 48 and 112.
 */
export function launcherLayout(width: number, height: number): LauncherLayout {
  const columns = width >= 700 ? 7 : 4;
  const gridWidth = Math.min(1160, Math.round(width * 0.78));
  const searchTop = Math.round(Math.min(40, Math.max(20, height * 0.035)));
  const dotsBottom = Math.round(Math.min(32, Math.max(16, height * 0.03)));
  // Below the search, its gap, and above the dots and theirs.
  const gridTop = searchTop + SEARCH_HEIGHT + Math.round(Math.min(48, height * 0.045));
  const available = height - gridTop - dotsBottom - 28;
  const labelSize = width >= 1600 ? 13 : 12;
  // Below an icon, the gap and its label's line; around both, the highlight's padding.
  const beside = 8 + labelSize + 6 + 2 * HIGHLIGHT_PADDING;
  const rows = [5, 4].find((n) => available / n >= LEAST_ICON + beside) ?? 3;
  const rowHeight = Math.max(LEAST_ICON + beside, Math.floor(available / rows));
  const roomForIcon = rowHeight - beside;
  const iconSize = Math.round(
    Math.max(LEAST_ICON, Math.min(112, (gridWidth / columns) * 0.56, roomForIcon)),
  );
  const searchWidth = Math.round(Math.min(280, Math.max(200, width * 0.19)));
  return {
    columns,
    rows,
    gridWidth,
    gridTop,
    rowHeight,
    iconSize,
    labelSize,
    searchTop,
    searchWidth,
    dotsBottom,
  };
}

/** The keys Launchpad answers, by their W3C names. */
export type LauncherKey = "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown";

/**
 * Where the highlight goes from `selected` (an index into all `count` entries, or null: none yet)
 * with `key`, pages of `columns` × `rows`: none yet, any arrow highlights the first icon of
 * `page`; left and right go through every icon, across pages; up and down go a row within the
 * page, staying where they are at its edge.
 */
export function moveSelection(
  selected: number | null,
  key: LauncherKey,
  count: number,
  columns: number,
  rows: number,
  page: number,
): number | null {
  if (count === 0) return null;
  const capacity = columns * rows;
  if (selected === null || selected >= count) {
    return Math.min(page * capacity, count - 1);
  }
  const start = Math.floor(selected / capacity) * capacity;
  const end = Math.min(count, start + capacity) - 1;
  switch (key) {
    case "ArrowLeft":
      return Math.max(0, selected - 1);
    case "ArrowRight":
      return Math.min(count - 1, selected + 1);
    case "ArrowUp":
      return selected - columns >= start ? selected - columns : selected;
    case "ArrowDown":
      return selected + columns <= end ? selected + columns : selected;
  }
}

/** The page an entry is on. */
export function pageOf(index: number, columns: number, rows: number): number {
  return Math.floor(index / (columns * rows));
}
