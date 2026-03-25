import { motion } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { useSessionStore } from "../../stores/sessionStore";

interface QuickSettingsProps {
  onClose: () => void;
}

function QuickSettings({ onClose }: QuickSettingsProps) {
  const {
    wifiEnabled,
    volume,
    brightness,
    batteryLevel,
    toggleWifi,
    setVolume,
    setBrightness,
  } = useSystemStore();

  return (
    <>
      {/* Click-away backdrop */}
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, y: 10, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 10, scale: 0.97 }}
        transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
        className="fixed bottom-[56px] right-2 z-50 w-[360px] origin-bottom-right rounded-2xl bg-white p-0 shadow-[0_8px_28px_rgba(0,0,0,0.16),0_2px_6px_rgba(0,0,0,0.06)]"
      >
        {/* Toggle pills grid */}
        <div className="grid grid-cols-2 gap-2 p-4 pb-3">
          {/* Wi-Fi */}
          <button
            onClick={toggleWifi}
            className={`flex items-center gap-3 rounded-2xl px-4 py-3.5 text-left transition-colors duration-150 ${
              wifiEnabled
                ? "bg-[#c2e7ff] text-[#001d35]"
                : "bg-[#e3e3e3] text-[#1f1f1f]"
            }`}
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" className="shrink-0">
              <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
            </svg>
            <div className="min-w-0">
              <div className="truncate text-[13px] font-medium leading-tight">Wi-Fi</div>
              <div className="truncate text-[11px] leading-tight opacity-60">
                {wifiEnabled ? "Connected" : "Off"}
              </div>
            </div>
          </button>

          {/* Bluetooth */}
          <button className="flex items-center gap-3 rounded-2xl bg-[#e3e3e3] px-4 py-3.5 text-left text-[#1f1f1f] transition-colors duration-150">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" className="shrink-0">
              <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
            </svg>
            <div className="min-w-0">
              <div className="truncate text-[13px] font-medium leading-tight">Bluetooth</div>
              <div className="truncate text-[11px] leading-tight opacity-60">On</div>
            </div>
          </button>

          {/* Do Not Disturb */}
          <button className="flex items-center gap-3 rounded-2xl bg-[#e3e3e3] px-4 py-3.5 text-left text-[#1f1f1f] transition-colors duration-150">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
              <line x1="1" y1="1" x2="23" y2="23" />
            </svg>
            <div className="truncate text-[13px] font-medium leading-tight">Do Not Disturb</div>
          </button>

          {/* Night Light */}
          <button className="flex items-center gap-3 rounded-2xl bg-[#e3e3e3] px-4 py-3.5 text-left text-[#1f1f1f] transition-colors duration-150">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
            </svg>
            <div className="truncate text-[13px] font-medium leading-tight">Night Light</div>
          </button>
        </div>

        {/* Sliders section */}
        <div className="border-t border-[#e0e0e0] px-5 pt-4 pb-3">
          {/* Brightness */}
          <div className="mb-3 flex items-center gap-4">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#5f6368" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <circle cx="12" cy="12" r="5" />
              <line x1="12" y1="1" x2="12" y2="3" />
              <line x1="12" y1="21" x2="12" y2="23" />
              <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
              <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
              <line x1="1" y1="12" x2="3" y2="12" />
              <line x1="21" y1="12" x2="23" y2="12" />
              <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
              <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
            </svg>
            <input
              type="range"
              min={0}
              max={100}
              value={brightness}
              onChange={(e) => setBrightness(Number(e.target.value))}
              className="h-2 flex-1 cursor-pointer appearance-none rounded-full outline-none [&::-moz-range-thumb]:h-5 [&::-moz-range-thumb]:w-5 [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-2 [&::-moz-range-thumb]:border-[#1a73e8] [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_3px_rgba(0,0,0,0.2)] [&::-webkit-slider-thumb]:h-5 [&::-webkit-slider-thumb]:w-5 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-2 [&::-webkit-slider-thumb]:border-[#1a73e8] [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_3px_rgba(0,0,0,0.2)]"
              style={{ background: `linear-gradient(to right, #1a73e8 ${brightness}%, #dadce0 ${brightness}%)` }}
            />
          </div>

          {/* Volume */}
          <div className="flex items-center gap-4">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="#5f6368" className="shrink-0">
              <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
            </svg>
            <input
              type="range"
              min={0}
              max={100}
              value={volume}
              onChange={(e) => setVolume(Number(e.target.value))}
              className="h-2 flex-1 cursor-pointer appearance-none rounded-full outline-none [&::-moz-range-thumb]:h-5 [&::-moz-range-thumb]:w-5 [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-2 [&::-moz-range-thumb]:border-[#1a73e8] [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_3px_rgba(0,0,0,0.2)] [&::-webkit-slider-thumb]:h-5 [&::-webkit-slider-thumb]:w-5 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-2 [&::-webkit-slider-thumb]:border-[#1a73e8] [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_3px_rgba(0,0,0,0.2)]"
              style={{ background: `linear-gradient(to right, #1a73e8 ${volume}%, #dadce0 ${volume}%)` }}
            />
          </div>
        </div>

        {/* Bottom bar */}
        <div className="flex items-center justify-between border-t border-[#e0e0e0] px-4 py-3">
          <span className="text-[12px] text-[#5f6368]">
            {batteryLevel}% — Charging
          </span>
          <div className="flex items-center gap-0.5">
            <button
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[#e8eaed]"
              aria-label="Settings"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#5f6368" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
            </button>
            <button
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[#e8eaed]"
              aria-label="Lock"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#5f6368" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="11" width="18" height="11" rx="2" />
                <path d="M7 11V7a5 5 0 0 1 10 0v4" />
              </svg>
            </button>
            <button
              onClick={() => useSessionStore.getState().logout()}
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[#fce8e6]"
              aria-label="Power off"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#5f6368" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
                <line x1="12" y1="2" x2="12" y2="12" />
              </svg>
            </button>
          </div>
        </div>
      </motion.div>
    </>
  );
}

export default QuickSettings;
