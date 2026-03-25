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
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
        <circle cx="12" cy="12" r="10" stroke="white" strokeWidth="2" />
        <ellipse cx="12" cy="12" rx="4" ry="10" stroke="white" strokeWidth="2" />
        <line x1="2" y1="12" x2="22" y2="12" stroke="white" strokeWidth="2" />
      </svg>
    ),
  },
  {
    name: "Files",
    color: "#4285f4",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
        <path
          d="M3 7V19C3 20.1 3.9 21 5 21H19C20.1 21 21 20.1 21 19V9C21 7.9 20.1 7 19 7H11L9 5H5C3.9 5 3 5.9 3 7Z"
          fill="white"
        />
      </svg>
    ),
  },
  {
    name: "Settings",
    color: "#5f6368",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
        <path
          d="M19.14 12.94A7.07 7.07 0 0019.2 12a7.07 7.07 0 00-.06-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96a6.9 6.9 0 00-1.62-.94l-.36-2.54A.48.48 0 0013.93 2h-3.86a.48.48 0 00-.48.41l-.36 2.54a6.9 6.9 0 00-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.71 8.81a.48.48 0 00.12.61l2.03 1.58a7.28 7.28 0 000 1.88L2.83 14.46a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.04.7 1.62.94l.36 2.54c.05.24.26.41.48.41h3.86c.22 0 .43-.17.48-.41l.36-2.54a6.9 6.9 0 001.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.6 3.6 0 0112 15.6z"
          fill="white"
        />
      </svg>
    ),
  },
  {
    name: "Terminal",
    color: "#202124",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
        <text x="4" y="17" fill="white" fontSize="14" fontFamily="monospace" fontWeight="bold">
          &gt;_
        </text>
      </svg>
    ),
  },
  {
    name: "Store",
    color: "#34a853",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
        <path
          d="M18 6H16C16 3.79 14.21 2 12 2S8 3.79 8 6H6C4.9 6 4 6.9 4 8V20C4 21.1 4.9 22 6 22H18C19.1 22 20 21.1 20 20V8C20 6.9 19.1 6 18 6ZM12 4C13.1 4 14 4.9 14 6H10C10 4.9 10.9 4 12 4Z"
          fill="white"
        />
      </svg>
    ),
  },
];

function LauncherButton() {
  const toggle = useLauncherStore((s) => s.toggle);

  return (
    <motion.button
      className="flex h-10 w-10 items-center justify-center rounded-full hover:bg-white/10"
      onClick={toggle}
      whileHover={{ scale: 1.1 }}
      whileTap={{ scale: 0.95 }}
      aria-label="App launcher"
    >
      <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
        {[0, 7, 14].map((x) =>
          [0, 7, 14].map((y) => (
            <circle key={`${x}-${y}`} cx={x + 3} cy={y + 3} r="2" fill="white" />
          )),
        )}
      </svg>
    </motion.button>
  );
}

function AppIcon({ app }: { app: PinnedApp }) {
  return (
    <motion.button
      className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full transition-transform hover:scale-110"
      style={{ backgroundColor: app.color }}
      whileTap={{ scale: [1, 1.2, 0.95, 1], transition: { duration: 0.4 } }}
      aria-label={app.name}
    >
      {app.icon}
    </motion.button>
  );
}

function WifiIcon({ enabled }: { enabled: boolean }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      {enabled ? (
        <path
          d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.24 4.24 0 00-6 0zm-4-4l2 2a7.07 7.07 0 0110 0l2-2C15.14 9.14 8.87 9.14 5 13z"
          fill="white"
        />
      ) : (
        <path
          d="M21 11l2-2c-3.73-3.73-8.87-5.15-13.7-4.31l2.58 2.58c3.3-.02 6.61 1.22 9.12 3.73zm-2 2a9.9 9.9 0 00-4.63-2.83l3.15 3.15.48-.32zM3.41 1.64L2 3.05 5.05 6.1C3.59 6.83 2.22 7.79 1 9l2 2c1.23-1.23 2.65-2.16 4.17-2.78l2.24 2.24A9.82 9.82 0 005 13l2 2a7.1 7.1 0 014.36-2.17l2.78 2.78A4.24 4.24 0 009 17l3 3 3-3c-.19-.19-.4-.36-.62-.51l5.56 5.56 1.41-1.41L3.41 1.64z"
          fill="#9aa0a6"
        />
      )}
    </svg>
  );
}

function BatteryIcon({ level }: { level: number }) {
  const fillWidth = Math.round((level / 100) * 14);
  const color = level <= 20 ? "#ea4335" : level <= 50 ? "#fbbc04" : "white";

  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <rect x="2" y="6" width="18" height="12" rx="2" stroke="white" strokeWidth="2" />
      <rect x="4" y="8" width={fillWidth} height="8" rx="1" fill={color} />
      <rect x="20" y="9" width="3" height="6" rx="1" fill="white" />
    </svg>
  );
}

export default function Shelf() {
  const time = useSystemStore((s) => s.time);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const [quickSettingsOpen, setQuickSettingsOpen] = useState(false);

  const toggleQuickSettings = useCallback(() => {
    setQuickSettingsOpen((prev) => !prev);
  }, []);

  return (
    <>
      <div
        className="fixed right-0 bottom-0 left-0 z-50 flex h-14 items-center px-3"
        style={{
          backgroundColor: "rgba(32, 33, 36, 0.72)",
          backdropFilter: "blur(64px) saturate(180%)",
          WebkitBackdropFilter: "blur(64px) saturate(180%)",
        }}
      >
        {/* Left: Launcher button */}
        <div className="flex items-center">
          <LauncherButton />
        </div>

        {/* Center: Pinned apps */}
        <div className="flex flex-1 items-center justify-center gap-2">
          {pinnedApps.map((app) => (
            <AppIcon key={app.name} app={app} />
          ))}
        </div>

        {/* Right: System tray */}
        <button
          className="flex cursor-pointer items-center gap-2 rounded-full px-3 py-1.5 hover:bg-white/10"
          onClick={toggleQuickSettings}
          aria-label="System tray"
        >
          <WifiIcon enabled={wifiEnabled} />
          <BatteryIcon level={batteryLevel} />
          <span className="text-[13px] font-medium text-white">{time || "--:--"}</span>
        </button>
      </div>

      {quickSettingsOpen && <QuickSettings onClose={() => setQuickSettingsOpen(false)} />}
    </>
  );
}
