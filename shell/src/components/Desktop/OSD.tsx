import { useEffect, useState, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";

export default function OSD() {
  const volume = useSystemStore((s) => s.volume);
  const brightness = useSystemStore((s) => s.brightness);
  const [visible, setVisible] = useState<"volume" | "brightness" | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const prevVolume = useRef(volume);
  const prevBrightness = useRef(brightness);
  const mounted = useRef(false);

  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    if (volume !== prevVolume.current) {
      prevVolume.current = volume;
      setVisible("volume");
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setVisible(null), 1500);
    }
  }, [volume]);

  useEffect(() => {
    if (!mounted.current) return;
    if (brightness !== prevBrightness.current) {
      prevBrightness.current = brightness;
      setVisible("brightness");
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setVisible(null), 1500);
    }
  }, [brightness]);

  const value = visible === "volume" ? volume : brightness;

  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          key="osd"
          initial={{ opacity: 0, y: -8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -8 }}
          transition={{ duration: 0.15 }}
          className="fixed top-6 left-1/2 z-[200] flex -translate-x-1/2 items-center gap-3 rounded-full border border-white/15 bg-white/12 px-5 py-2.5 shadow-[0_4px_20px_rgba(0,0,0,0.3)] backdrop-blur-[60px]"
        >
          {visible === "volume" ? (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.7">
              <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
            </svg>
          ) : (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.7">
              <circle cx="12" cy="12" r="5" />
              <line x1="12" y1="1" x2="12" y2="3" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
              <line x1="12" y1="21" x2="12" y2="23" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
              <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
              <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
              <line x1="1" y1="12" x2="3" y2="12" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
              <line x1="21" y1="12" x2="23" y2="12" stroke="white" strokeOpacity="0.7" strokeWidth="2" strokeLinecap="round" />
            </svg>
          )}
          <div className="h-[4px] w-[120px] overflow-hidden rounded-full bg-white/15">
            <div
              className="h-full rounded-full bg-white/80 transition-all duration-100"
              style={{ width: `${value}%` }}
            />
          </div>
          <span className="min-w-[28px] text-right text-[11px] font-medium text-white/60">
            {value}%
          </span>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
