import type { SurfaceFn } from "./surfaceEquations";

/**
 * Phase 1: Calculate 1D displacement profile along the bezel.
 * Returns an array of displacement magnitudes for each sample point.
 */
export function calculateDisplacementProfile(
  glassThickness: number,
  bezelWidth: number,
  surfaceFn: SurfaceFn,
  refractiveIndex: number,
  samples = 128,
): number[] {
  const eta = 1 / refractiveIndex;

  function refract(normalX: number, normalY: number): [number, number] | null {
    const dot = normalY;
    const k = 1 - eta * eta * (1 - dot * dot);
    if (k < 0) return null; // Total internal reflection
    const kSqrt = Math.sqrt(k);
    return [
      -(eta * dot + kSqrt) * normalX,
      eta - (eta * dot + kSqrt) * normalY,
    ];
  }

  return Array.from({ length: samples }, (_, i) => {
    const x = i / samples;
    const y = surfaceFn.fn(x);

    const dx = x < 1 ? 0.0001 : -0.0001;
    const y2 = surfaceFn.fn(x + dx);
    const derivative = (y2 - y) / dx;
    const magnitude = Math.sqrt(derivative * derivative + 1);
    const normal: [number, number] = [-derivative / magnitude, -1 / magnitude];
    const refracted = refract(normal[0], normal[1]);

    if (!refracted) return 0;

    const remainingHeightOnBezel = y * bezelWidth;
    const remainingHeight = remainingHeightOnBezel + glassThickness;
    return refracted[0] * (remainingHeight / refracted[1]);
  });
}

export type ShapeType = "circle" | "squircle" | "rectangle" | "pill";

/**
 * Phase 2: Generate a 2D displacement map ImageData from the 1D profile.
 * Supports various shapes via squircle distance function.
 */
export function calculateDisplacementMap(
  objectWidth: number,
  objectHeight: number,
  bezelWidth: number,
  profile: number[],
  shape: ShapeType = "pill",
  cornerRadius = 1.0,
  squircleExponent = 4,
  dpr?: number,
): { imageData: ImageData; maxDisplacement: number } {
  const devicePixelRatio = dpr ?? window.devicePixelRatio ?? 1;
  const bufferWidth = Math.floor(objectWidth * devicePixelRatio);
  const bufferHeight = Math.floor(objectHeight * devicePixelRatio);

  const imageData = new ImageData(bufferWidth, bufferHeight);
  // Fill neutral (128, 128, 0, 255)
  const neutral = 0xff008080;
  new Uint32Array(imageData.data.buffer).fill(neutral);

  const bezel = bezelWidth * devicePixelRatio;
  const objectW = bufferWidth;
  const objectH = bufferHeight;

  const maxCornerRadius = Math.min(objectW, objectH) / 2;
  let radius_: number;

  switch (shape) {
    case "circle":
      radius_ = maxCornerRadius;
      break;
    case "pill":
      radius_ = Math.min(objectW, objectH) / 2;
      break;
    case "rectangle":
    case "squircle":
    default:
      radius_ = cornerRadius * maxCornerRadius;
      break;
  }

  const radiusSquared = radius_ ** 2;
  const radiusPlusOneSquared = (radius_ + 1) ** 2;
  const radiusMinusBezelSquared = Math.max(0, (radius_ - bezel) ** 2);
  const widthBetweenRadiuses = Math.max(0, objectW - radius_ * 2);
  const heightBetweenRadiuses = Math.max(0, objectH - radius_ * 2);

  const squircleDistance = (x: number, y: number, r: number, n: number): number => {
    if (r === 0) return Math.sqrt(x * x + y * y);
    const absX = Math.abs(x) / r;
    const absY = Math.abs(y) / r;
    return Math.pow(Math.pow(absX, n) + Math.pow(absY, n), 1 / n) * r;
  };

  const maxDisp = Math.max(...profile.map((x) => Math.abs(x))) || 1;

  for (let y1 = 0; y1 < objectH; y1++) {
    for (let x1 = 0; x1 < objectW; x1++) {
      const idx = (y1 * objectW + x1) * 4;

      const isOnLeftSide = x1 < radius_;
      const isOnRightSide = x1 >= objectW - radius_;
      const isOnTopSide = y1 < radius_;
      const isOnBottomSide = y1 >= objectH - radius_;

      let x = 0, y = 0;
      let distanceToEdge = 0;
      let normalX = 0, normalY = 0;
      let isInBezelRegion = false;

      if ((isOnLeftSide || isOnRightSide) && (isOnTopSide || isOnBottomSide)) {
        // Corner region
        x = isOnLeftSide ? x1 - radius_ : x1 - (objectW - radius_);
        y = isOnTopSide ? y1 - radius_ : y1 - (objectH - radius_);

        let distFromCorner: number;
        if (shape === "squircle" && cornerRadius > 0) {
          distFromCorner = squircleDistance(x, y, radius_, squircleExponent);
        } else {
          distFromCorner = Math.sqrt(x * x + y * y);
        }

        distanceToEdge = radius_ - distFromCorner;
        if (distanceToEdge >= -1 && distanceToEdge <= bezel) {
          isInBezelRegion = true;
          const mag = Math.sqrt(x * x + y * y) || 1;
          normalX = x / mag;
          normalY = y / mag;
        }
      } else if (isOnLeftSide || isOnRightSide) {
        distanceToEdge = isOnLeftSide ? x1 : objectW - 1 - x1;
        if (distanceToEdge <= bezel) {
          isInBezelRegion = true;
          normalX = isOnLeftSide ? -1 : 1;
          normalY = 0;
        }
      } else if (isOnTopSide || isOnBottomSide) {
        distanceToEdge = isOnTopSide ? y1 : objectH - 1 - y1;
        if (distanceToEdge <= bezel) {
          isInBezelRegion = true;
          normalX = 0;
          normalY = isOnTopSide ? -1 : 1;
        }
      }

      if (isInBezelRegion && distanceToEdge >= 0) {
        const opacity = distanceToEdge >= 0 ? 1 : Math.max(0, 1 + distanceToEdge);

        const bezelIndex = Math.min(
          profile.length - 1,
          Math.max(0, ((distanceToEdge / bezel) * profile.length) | 0),
        );
        const distance = profile[bezelIndex] ?? 0;

        const dX = (-normalX * distance) / maxDisp;
        const dY = (-normalY * distance) / maxDisp;

        imageData.data[idx] = 128 + dX * 127 * opacity;
        imageData.data[idx + 1] = 128 + dY * 127 * opacity;
        imageData.data[idx + 2] = 0;
        imageData.data[idx + 3] = 255;
      }
    }
  }

  return { imageData, maxDisplacement: maxDisp };
}
