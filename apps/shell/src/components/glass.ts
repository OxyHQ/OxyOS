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
