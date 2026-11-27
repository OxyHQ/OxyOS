/**
 * The shell's glass (docs/adr/0019-settings.mdx, `reduce_transparency`): the fills of its panels,
 * banners and sheets are a Mac's materials, translucent over what is behind them; with Reduce
 * transparency on they are drawn opaque, the same colour at full alpha, as a Mac draws its
 * materials then. A dim over the desktop (a backdrop that darkens what is behind it) is not glass
 * and stays a dim. Pure, so it is tested without a renderer.
 */
export function glass(fill: string, reduced: boolean): string {
  if (!reduced) return fill;
  const hex = /^#([0-9a-f]{6})(?:[0-9a-f]{2})?$/i.exec(fill);
  if (hex) return `#${hex[1].toLowerCase()}`;
  const rgba = /^rgba\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*[\d.]+\s*\)$/i.exec(fill);
  if (rgba) return `rgb(${rgba[1]}, ${rgba[2]}, ${rgba[3]})`;
  return fill;
}

/**
 * How much of an opaque fill the tint over the platform's material keeps: Bloom's floating
 * surfaces' (`SURFACE_TINT_OPACITY`), the rest the blurred backdrop showing through.
 */
export const MATERIAL_TINT_OPACITY = 0.72;

/**
 * A glass fill as the tint over the platform's material (docs/adr/0069-the-platforms-material.mdx),
 * as Bloom tints a floating surface: an explicit alpha is kept, and an opaque fill (the dock's
 * shelf) keeps MATERIAL_TINT_OPACITY of itself. Only while the material is drawn, so never with
 * Reduce transparency on.
 */
export function materialTint(fill: string): string {
  if (/^#[0-9a-f]{6}$/i.test(fill)) {
    const alpha = Math.round(MATERIAL_TINT_OPACITY * 255)
      .toString(16)
      .padStart(2, "0");
    return `${fill.toLowerCase()}${alpha}`;
  }
  return fill;
}
