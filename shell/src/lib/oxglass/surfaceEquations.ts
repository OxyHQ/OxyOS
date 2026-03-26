export interface SurfaceFn {
  name: string;
  fn: (x: number) => number;
}

/** Spherical dome — sharper refraction at edges */
export const CONVEX_CIRCLE: SurfaceFn = {
  name: "Convex Circle",
  fn: (x) => Math.sqrt(1 - (1 - x) ** 2),
};

/** Squircle dome — softer flat→curve transition, ideal for rectangular panels */
export const CONVEX_SQUIRCLE: SurfaceFn = {
  name: "Convex Squircle",
  fn: (x) => Math.pow(1 - Math.pow(1 - x, 4), 1 / 4),
};

/** Concave — diverges light outward */
export const CONCAVE: SurfaceFn = {
  name: "Concave",
  fn: (x) => 1 - CONVEX_CIRCLE.fn(x),
};

/** Raised rim with shallow center dip — good for buttons */
export const LIP: SurfaceFn = {
  name: "Lip",
  fn: (x) => {
    const convex = CONVEX_SQUIRCLE.fn(x * 2);
    const concave = CONCAVE.fn(x) + 0.1;
    const smootherstep = 6 * x ** 5 - 15 * x ** 4 + 10 * x ** 3;
    return convex * (1 - smootherstep) + concave * smootherstep;
  },
};

export const SURFACES: Record<string, SurfaceFn> = {
  convex: CONVEX_SQUIRCLE,
  circle: CONVEX_CIRCLE,
  concave: CONCAVE,
  lip: LIP,
};
