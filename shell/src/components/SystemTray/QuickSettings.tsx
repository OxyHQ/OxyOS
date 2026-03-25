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
    time,
    toggleWifi,
    setVolume,
    setBrightness,
  } = useSystemStore();

  const formattedDate = new Date().toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <motion.div
        initial={{ opacity: 0, y: 20, scale: 0.95 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 20, scale: 0.95 }}
        transition={{ duration: 0.2, ease: [0.2, 0, 0, 1] }}
        className="fixed right-2 z-50 w-[360px] origin-bottom-right rounded-[20px] p-5"
      style={{
        bottom: 64,
        background: "rgba(32, 33, 36, 0.92)",
        backdropFilter: "blur(64px) saturate(180%)",
        WebkitBackdropFilter: "blur(64px) saturate(180%)",
        boxShadow:
          "0 8px 32px rgba(0, 0, 0, 0.4), 0 2px 8px rgba(0, 0, 0, 0.2)",
      }}
    >
      {/* Header */}
      <div className="mb-5 flex items-start justify-between">
        <div>
          <p className="text-[32px] font-medium leading-tight text-white">
            {time || "--:--"}
          </p>
          <p className="mt-0.5 text-sm text-[#9aa0a6]">{formattedDate}</p>
        </div>
        <button
          onClick={() => useSessionStore.getState().logout()}
          className="flex h-10 w-10 items-center justify-center rounded-full transition-colors hover:bg-red-500/20"
          aria-label="Power off"
        >
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="#ea4335"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
            <line x1="12" y1="2" x2="12" y2="12" />
          </svg>
        </button>
      </div>

      {/* Quick toggles */}
      <div className="mb-5 flex gap-2">
        <button
          onClick={toggleWifi}
          className="flex items-center gap-2 rounded-full px-4 py-2 text-sm font-medium transition-colors"
          style={{
            background: wifiEnabled
              ? "rgba(26, 115, 232, 0.9)"
              : "rgba(95, 99, 104, 0.4)",
            color: wifiEnabled ? "#ffffff" : "#dadce0",
          }}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="currentColor"
          >
            <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
          </svg>
          Wi-Fi
        </button>
        <button
          className="flex items-center gap-2 rounded-full px-4 py-2 text-sm font-medium transition-colors"
          style={{
            background: "rgba(95, 99, 104, 0.4)",
            color: "#dadce0",
          }}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="currentColor"
          >
            <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
          </svg>
          Bluetooth
        </button>
      </div>

      {/* Brightness slider */}
      <div className="mb-3 flex items-center gap-3">
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="#9aa0a6"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="shrink-0"
        >
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
          className="slider h-2 w-full cursor-pointer appearance-none rounded-full bg-white/20 outline-none [&::-moz-range-thumb]:h-4 [&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-none [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)] [&::-webkit-slider-thumb]:h-4 [&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)]"
          style={{
            background: `linear-gradient(to right, #1a73e8 ${brightness}%, rgba(255,255,255,0.2) ${brightness}%)`,
          }}
        />
      </div>

      {/* Volume slider */}
      <div className="flex items-center gap-3">
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="#9aa0a6"
          className="shrink-0"
        >
          <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
        </svg>
        <input
          type="range"
          min={0}
          max={100}
          value={volume}
          onChange={(e) => setVolume(Number(e.target.value))}
          className="slider h-2 w-full cursor-pointer appearance-none rounded-full bg-white/20 outline-none [&::-moz-range-thumb]:h-4 [&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-none [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)] [&::-webkit-slider-thumb]:h-4 [&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)]"
          style={{
            background: `linear-gradient(to right, #1a73e8 ${volume}%, rgba(255,255,255,0.2) ${volume}%)`,
          }}
        />
      </div>
    </motion.div>
    </>
  );
}

export default QuickSettings;
