/** Shared glass/frosted style constants for consistent UI across components */

export const glass = {
  panel: "rounded-[18px] border border-white/15 bg-white/12 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[70px] backdrop-saturate-[200%]",
  menu: "rounded-[14px] border border-white/15 bg-white/12 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[70px] backdrop-saturate-[200%]",
  quickSettings: "rounded-[18px] border border-white/15 bg-white/12 shadow-[0_4px_24px_rgba(0,0,0,0.25),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[60px] backdrop-saturate-[180%]",
};

/** Battery fill computation shared between Shelf and QuickSettings */
export function getBatteryVisuals(level: number, isCharging: boolean) {
  return {
    fillWidth: Math.round((level / 100) * 18),
    fillColor: isCharging ? "#30d158" : level <= 20 ? "#ff453a" : "white",
  };
}
