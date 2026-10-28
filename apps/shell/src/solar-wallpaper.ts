export type SolarAnchor = {
  frame: number;
  elevationDegrees: number;
  azimuthDegrees: number;
};

/**
 * Select from the original solar-elevation anchors, distinguishing morning from
 * evening. The system must supply solar position; this module never requests
 * location, assumes a latitude, or fabricates sunrise/sunset clock times.
 * Midnight is eligible on either side of the cycle.
 */
export function selectSolarFrame(
  anchors: readonly SolarAnchor[],
  elevationDegrees: number,
  rising: boolean,
): number {
  if (
    !Number.isFinite(elevationDegrees) ||
    elevationDegrees < -90 ||
    elevationDegrees > 90
  )
    throw new RangeError("Solar elevation must be between -90 and 90 degrees");
  const minimum = Math.min(...anchors.map((anchor) => anchor.elevationDegrees));
  const maximum = Math.max(...anchors.map((anchor) => anchor.elevationDegrees));
  const candidates = anchors.filter(
    (anchor) =>
      anchor.elevationDegrees === minimum ||
      anchor.elevationDegrees === maximum ||
      anchor.azimuthDegrees < 180 === rising,
  );
  if (!candidates.length) throw new Error("No solar anchors for this phase");
  return candidates.reduce((closest, anchor) =>
    Math.abs(anchor.elevationDegrees - elevationDegrees) <
    Math.abs(closest.elevationDegrees - elevationDegrees)
      ? anchor
      : closest,
  ).frame;
}
