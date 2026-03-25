import { useState, useCallback } from "react";
import { motion } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";
import { useSystemStore } from "../../stores/systemStore";
import QuickSettings from "../SystemTray/QuickSettings";

interface PinnedApp {
  name: string;
  color: string;
  icon: React.ReactNode;
}

const pinnedApps: PinnedApp[] = [
  {
    name: "Browser",
    color: "#4285f4",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="none">
        <circle cx="12" cy="12" r="10" stroke="white" strokeWidth="1.5" />
        <ellipse cx="12" cy="12" rx="4" ry="10" stroke="white" strokeWidth="1.5" />
        <line x1="2" y1="12" x2="22" y2="12" stroke="white" strokeWidth="1.5" />
      </svg>
    ),
  },
  {
    name: "Files",
    color: "#4285f4",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M3 7V19C3 20.1 3.9 21 5 21H19C20.1 21 21 20.1 21 19V9C21 7.9 20.1 7 19 7H11L9 5H5C3.9 5 3 5.9 3 7Z" />
      </svg>
    ),
  },
  {
    name: "Settings",
    color: "#5f6368",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54A.48.48 0 0013.93 2h-3.86a.48.48 0 00-.48.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.71 8.81a.48.48 0 00.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.26.41.48.41h3.86c.22 0 .43-.17.48-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.6 3.6 0 0112 15.6z" />
      </svg>
    ),
  },
  {
    name: "Terminal",
    color: "#202124",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
        <path d="M7 8l4 4-4 4" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        <line x1="13" y1="16" x2="17" y2="16" stroke="white" strokeWidth="2" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    name: "Store",
    color: "#34a853",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M18 6h-2c0-2.21-1.79-4-4-4S8 3.79 8 6H6c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm-6-2c1.1 0 2 .9 2 2h-4c0-1.1.9-2 2-2z" />
      </svg>
    ),
  },
];

function WifiIcon({ enabled }: { enabled: boolean }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill={enabled ? "white" : "#9aa0a6"}>
      <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.24 4.24 0 00-6 0zm-4-4l2 2a7.07 7.07 0 0110 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
    </svg>
  );
}

function BatteryIcon({ level }: { level: number }) {
  const fillWidth = Math.round((level / 100) * 12);
  const color = level <= 20 ? "#ea4335" : level <= 50 ? "#fbbc04" : "white";

  return (
    <svg width="18" height="10" viewBox="0 0 20 12" fill="none">
      <rect x="0.75" y="0.75" width="15.5" height="10.5" rx="2" stroke="white" strokeWidth="1.5" />
      <rect x="2.5" y="2.5" width={fillWidth} height="7" rx="1" fill={color} />
      <path d="M17.5 4.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5" stroke="white" strokeWidth="1.5" />
    </svg>
  );
}

export default function Shelf() {
  const time = useSystemStore((s) => s.time);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const [quickSettingsOpen, setQuickSettingsOpen] = useState(false);
  const toggleLauncher = useLauncherStore((s) => s.toggle);

  const toggleQuickSettings = useCallback(() => {
    setQuickSettingsOpen((prev) => !prev);
  }, []);

  const shortDate = new Date().toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });

  return (
    <>
      {/* Full-width shelf bar — rounded top only like ChromeOS */}
      <div className="fixed right-0 bottom-0 left-0 z-40 grid h-[48px] grid-cols-[auto_1fr_auto] items-center rounded-t-[16px] bg-[rgba(32,33,36,0.82)] px-3 backdrop-blur-[64px] backdrop-saturate-[180%]">
        {/* Left: Launcher button */}
        <button
          className="flex h-10 w-10 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
          onClick={toggleLauncher}
          aria-label="App launcher"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="white">
            <circle cx="5" cy="5" r="2.5" />
            <circle cx="12" cy="5" r="2.5" />
            <circle cx="19" cy="5" r="2.5" />
            <circle cx="5" cy="12" r="2.5" />
            <circle cx="12" cy="12" r="2.5" />
            <circle cx="19" cy="12" r="2.5" />
            <circle cx="5" cy="19" r="2.5" />
            <circle cx="12" cy="19" r="2.5" />
            <circle cx="19" cy="19" r="2.5" />
          </svg>
        </button>

        {/* Center: Pinned apps */}
        <div className="flex items-center justify-center gap-1">
          {pinnedApps.map((app) => (
            <motion.button
              key={app.name}
              className="flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center rounded-full transition-all duration-150 hover:scale-110 hover:bg-white/10"
              whileTap={{ scale: [1, 1.2, 0.95, 1], transition: { duration: 0.4 } }}
              aria-label={app.name}
            >
              <div
                className="flex h-10 w-10 items-center justify-center rounded-full"
                style={{ backgroundColor: app.color }}
              >
                {app.icon}
              </div>
            </motion.button>
          ))}
        </div>

        {/* Right: System tray */}
        <button
          className="flex shrink-0 cursor-pointer items-center gap-2.5 rounded-full px-3 py-1.5 transition-colors duration-150 hover:bg-white/10"
          onClick={toggleQuickSettings}
          aria-label="System tray"
        >
          <span className="text-[12px] font-medium leading-none text-white">
            {shortDate}
          </span>
          <span className="text-[12px] font-medium leading-none text-white">
            {time || "--:--"}
          </span>
          <WifiIcon enabled={wifiEnabled} />
          <BatteryIcon level={batteryLevel} />
        </button>
      </div>

      {quickSettingsOpen && (
        <QuickSettings onClose={() => setQuickSettingsOpen(false)} />
      )}
    </>
  );
}
