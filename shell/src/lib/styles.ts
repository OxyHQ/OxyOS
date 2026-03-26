/** Shared glass/frosted style constants for consistent UI across components */

export const glass = {
  panel: "rounded-[18px] border border-white/15 bg-white/12 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[70px] backdrop-saturate-[200%]",
  floatingPanel: "rounded-[20px] border border-white/20 bg-white/12 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]",
  menu: "rounded-[14px] border border-white/15 bg-white/12 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[70px] backdrop-saturate-[200%]",
  quickSettings: "rounded-[18px] border border-white/15 bg-white/12 shadow-[0_4px_24px_rgba(0,0,0,0.25),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[60px] backdrop-saturate-[180%]",
};

export const sliderThumb =
  "[&::-webkit-slider-thumb]:h-[18px] [&::-webkit-slider-thumb]:w-[18px] [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-0 [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_0_6px_rgba(0,0,0,0.3)] [&::-moz-range-thumb]:h-[18px] [&::-moz-range-thumb]:w-[18px] [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_0_6px_rgba(0,0,0,0.3)]";

/** OxGlass presets — values match the original vue-web-liquid-glass repo */
export const oxGlassPresets = {
  /** Panels: notifications, settings, calendar */
  panel: { surface: "convex" as const, bezel: 40, glassThickness: 120, refraction: 1.5, blur: 0.2, scaleRatio: 1, specularOpacity: 0.4, specularSaturation: 4, shape: "pill" as const },
  /** Floating panels with stronger effect */
  floatingPanel: { surface: "convex" as const, bezel: 40, glassThickness: 120, refraction: 1.5, blur: 0.2, scaleRatio: 1, specularOpacity: 0.5, specularSaturation: 6, shape: "pill" as const },
  /** Shelf / bottom bar — matches original BottomNavBar background */
  shelf: { surface: "convex" as const, bezel: 30, glassThickness: 190, refraction: 1.3, blur: 2, scaleRatio: 0.4, specularOpacity: 1, specularSaturation: 19, shape: "pill" as const },
  /** Slider/switch thumbs */
  thumb: { surface: "convex" as const, bezel: 8, glassThickness: 100, refraction: 1.5, blur: 0, scaleRatio: 0.1, specularOpacity: 0.4, specularSaturation: 7, shape: "pill" as const },
  /** Switch thumb (lip profile) */
  switchThumb: { surface: "lip" as const, bezel: 16, glassThickness: 20, refraction: 1.5, blur: 0.2, scaleRatio: 0.4, specularOpacity: 0.5, specularSaturation: 6, shape: "pill" as const },
  /** Toast / pill-shaped elements */
  toast: { surface: "convex" as const, bezel: 30, glassThickness: 120, refraction: 1.4, blur: 0.2, scaleRatio: 0.8, specularOpacity: 0.4, specularSaturation: 4, shape: "pill" as const },
};

/** Battery fill computation shared between Shelf and QuickSettings */
export function getBatteryVisuals(level: number, isCharging: boolean) {
  return {
    fillWidth: Math.round((level / 100) * 18),
    fillColor: isCharging ? "#30d158" : level <= 20 ? "#ff453a" : "white",
  };
}
