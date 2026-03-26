import { motion } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { useWidgetStore } from "../../stores/widgetStore";
import { glass } from "../../lib/styles";

const widgetAnimation = {
  initial: { opacity: 0, y: 12 },
  animate: { opacity: 1, y: 0 },
  transition: { duration: 0.45, ease: [0.22, 1, 0.36, 1] },
};

function ClockWidget() {
  const time = useSystemStore((s) => s.time);

  const now = new Date();
  const dateString = now.toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  return (
    <motion.div
      className={`${glass.panel} w-[200px] px-5 py-4`}
      {...widgetAnimation}
      transition={{ ...widgetAnimation.transition, delay: 0 }}
    >
      <p className="text-[36px] font-bold leading-tight tracking-tight text-white">
        {time || "--:--"}
      </p>
      <p className="mt-0.5 text-[13px] leading-snug text-white/60">
        {dateString}
      </p>
    </motion.div>
  );
}

function WeatherWidget() {
  return (
    <motion.div
      className={`${glass.panel} flex w-[200px] items-center gap-3 px-5 py-4`}
      {...widgetAnimation}
      transition={{ ...widgetAnimation.transition, delay: 0.08 }}
    >
      {/* Sun + cloud SVG icon */}
      <svg
        width="40"
        height="40"
        viewBox="0 0 40 40"
        fill="none"
        className="shrink-0"
      >
        {/* Sun */}
        <circle cx="18" cy="16" r="6" fill="#FFD60A" />
        <g stroke="#FFD60A" strokeWidth="1.5" strokeLinecap="round">
          <line x1="18" y1="5" x2="18" y2="7.5" />
          <line x1="18" y1="24.5" x2="18" y2="27" />
          <line x1="7" y1="16" x2="9.5" y2="16" />
          <line x1="26.5" y1="16" x2="29" y2="16" />
          <line x1="10.2" y1="8.2" x2="12" y2="10" />
          <line x1="24" y1="22" x2="25.8" y2="23.8" />
          <line x1="25.8" y1="8.2" x2="24" y2="10" />
          <line x1="12" y1="22" x2="10.2" y2="23.8" />
        </g>
        {/* Cloud */}
        <path
          d="M14 30a5 5 0 0 1-.3-10h.3a7 7 0 0 1 13.4 2.8A4 4 0 0 1 30 30H14z"
          fill="white"
          fillOpacity="0.85"
        />
      </svg>

      <div className="min-w-0">
        <p className="text-[22px] font-semibold leading-tight text-white">
          22°C
        </p>
        <p className="text-[12px] leading-snug text-white/60">Partly Cloudy</p>
      </div>
    </motion.div>
  );
}

function QuickNotesWidget() {
  const notes = useWidgetStore((s) => s.notes);
  const setNotes = useWidgetStore((s) => s.setNotes);

  return (
    <motion.div
      className={`${glass.panel} w-[200px] px-5 py-4`}
      {...widgetAnimation}
      transition={{ ...widgetAnimation.transition, delay: 0.16 }}
    >
      <p className="mb-2 text-[13px] font-semibold text-white/70">
        Quick Notes
      </p>
      <textarea
        rows={3}
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
        placeholder="Type a note..."
        className="w-full resize-none rounded-lg border border-white/10 bg-white/8 px-3 py-2 text-[13px] leading-snug text-white placeholder-white/30 outline-none focus:border-white/25"
      />
    </motion.div>
  );
}

export default function Widgets() {
  return (
    <div className="absolute left-6 top-6 z-10 flex flex-col gap-3">
      <ClockWidget />
      <WeatherWidget />
      <QuickNotesWidget />
    </div>
  );
}
