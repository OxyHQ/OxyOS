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
