import { useMemo } from "react";
import { motion } from "framer-motion";
import { useOxGlass } from "../../hooks/useOxGlass";
import OxGlassFilter from "../shared/OxGlassFilter";
import GradientBlur from "../shared/GradientBlur";
import { oxGlassPresets } from "../../lib/styles";

interface CalendarPopupProps {
  onClose: () => void;
}

const DAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

export default function CalendarPopup({ onClose }: CalendarPopupProps) {
  const now = new Date();
  const year = now.getFullYear();
  const month = now.getMonth();
  const today = now.getDate();

  const { firstDay, daysInMonth } = useMemo(() => {
    const first = new Date(year, month, 1).getDay();
    const dim = new Date(year, month + 1, 0).getDate();
    return { firstDay: first, daysInMonth: dim };
  }, [year, month]);

  const monthName = now.toLocaleDateString("en-US", { month: "long", year: "numeric" });

  const cells: (number | null)[] = [];
  for (let i = 0; i < firstDay; i++) cells.push(null);
  for (let d = 1; d <= daysInMonth; d++) cells.push(d);

  const oxglass = useOxGlass(oxGlassPresets.panel);

  return (
    <>
      {oxglass.filterData && (
        <OxGlassFilter
          filterId={oxglass.filterId}
          filterData={oxglass.filterData}
          blur={oxglass.blur}
          specularOpacity={oxglass.specularOpacity}
          specularSaturation={oxglass.specularSaturation}
        />
      )}

      <div className="fixed inset-0 z-40" onClick={onClose} />
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 8 }}
        transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
        className="fixed right-2 bottom-[64px] z-50 w-[280px] origin-bottom-right"
      >
      <div ref={oxglass.ref} className="rounded-[18px] border border-white/15 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] p-4" style={{ ...oxglass.glassStyle, overflow: "hidden", position: "relative" }}>
        <GradientBlur direction="top" size={20} />
        <GradientBlur direction="bottom" size={20} />
        <GradientBlur direction="left" size={20} />
        <GradientBlur direction="right" size={20} />
        {/* Month header */}
        <p className="mb-3 text-center text-[13px] font-semibold text-white/90">
          {monthName}
        </p>

        {/* Day headers */}
        <div className="mb-1 grid grid-cols-7 gap-0">
          {DAYS.map((d) => (
            <div key={d} className="py-1 text-center text-[10px] font-semibold text-white/35">
              {d}
            </div>
          ))}
        </div>

        {/* Day cells */}
        <div className="grid grid-cols-7 gap-0">
          {cells.map((day, i) => (
            <div key={i} className="flex items-center justify-center py-[3px]">
              {day ? (
                <div
                  className={`flex h-[28px] w-[28px] items-center justify-center rounded-full text-[12px] font-medium ${
                    day === today
                      ? "bg-[#0a84ff] text-white"
                      : "text-white/70"
                  }`}
                >
                  {day}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      </div>
      </motion.div>
    </>
  );
}
