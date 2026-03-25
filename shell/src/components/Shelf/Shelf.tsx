import { useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
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
    name: "Chrome",
    color: "#4285f4",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
        <circle cx="12" cy="12" r="10" stroke="white" strokeWidth="1.5" />
        <ellipse cx="12" cy="12" rx="4" ry="10" stroke="white" strokeWidth="1.5" />
        <line x1="2" y1="12" x2="22" y2="12" stroke="white" strokeWidth="1.5" />
      </svg>
    ),
  },
  {
    name: "Reddit",
    color: "#ff4500",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <circle cx="12" cy="14" r="7" />
        <circle cx="9" cy="13" r="1.2" fill="#ff4500" />
        <circle cx="15" cy="13" r="1.2" fill="#ff4500" />
        <path d="M9.5 16.5c1 1 4 1 5 0" stroke="#ff4500" strokeWidth="1" fill="none" strokeLinecap="round" />
        <line x1="12" y1="7" x2="14" y2="3" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
        <circle cx="15" cy="3" r="1.5" fill="white" />
      </svg>
    ),
  },
  {
    name: "Discord",
    color: "#5865f2",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M9.5 3h5l1 2h3.5v13h-5l-2 3-2-3H4.5V5H8l1.5-2zM8 10v3M12 9v4M16 10v3" stroke="white" strokeWidth="1.5" strokeLinecap="round" fill="none" />
      </svg>
    ),
  },
  {
    name: "Twitter",
    color: "#1da1f2",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M22 5.8a8.5 8.5 0 01-2.36.64 4.13 4.13 0 001.81-2.27 8.2 8.2 0 01-2.6 1 4.1 4.1 0 00-7 3.74A11.6 11.6 0 013.4 4.5a4.1 4.1 0 001.27 5.48A4 4 0 013 9.5v.05a4.1 4.1 0 003.29 4 4 4 0 01-1.85.07 4.1 4.1 0 003.83 2.85A8.23 8.23 0 012 18.4a11.6 11.6 0 006.29 1.84c7.55 0 11.67-6.25 11.67-11.67 0-.18 0-.35-.01-.53A8.35 8.35 0 0022 5.8z" />
      </svg>
    ),
  },
  {
    name: "Chrome Web Store",
    color: "#4285f4",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M18 6h-2c0-2.21-1.79-4-4-4S8 3.79 8 6H6c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm-6-2c1.1 0 2 .9 2 2h-4c0-1.1.9-2 2-2z" />
      </svg>
    ),
  },
  {
    name: "Spotify",
    color: "#1db954",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
        <path d="M8 15c4-1 8-.5 10 .5" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
        <path d="M7 12c5-1.5 10-.5 12 1" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
        <path d="M6 9c6-2 12-.5 14 1.5" stroke="white" strokeWidth="2" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    name: "Files",
    color: "#4285f4",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M3 7V19C3 20.1 3.9 21 5 21H19C20.1 21 21 20.1 21 19V9C21 7.9 20.1 7 19 7H11L9 5H5C3.9 5 3 5.9 3 7Z" />
      </svg>
    ),
  },
  {
    name: "YouTube",
    color: "#ff0000",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M10 8.5v7l6-3.5-6-3.5z" />
        <rect x="3" y="5" width="18" height="14" rx="3" stroke="white" strokeWidth="1.5" fill="none" />
      </svg>
    ),
  },
  {
    name: "Gmail",
    color: "#ea4335",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="none">
        <rect x="3" y="5" width="18" height="14" rx="2" stroke="white" strokeWidth="1.5" />
        <path d="M3 7l9 6 9-6" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
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
    name: "Maps",
    color: "#34a853",
    icon: (
      <svg width="22" height="22" viewBox="0 0 24 24" fill="white">
        <path d="M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7zm0 9.5a2.5 2.5 0 110-5 2.5 2.5 0 010 5z" />
      </svg>
    ),
  },
];

export default function Shelf() {
  const time = useSystemStore((s) => s.time);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const [quickSettingsOpen, setQuickSettingsOpen] = useState(false);
  const toggleLauncher = useLauncherStore((s) => s.toggle);

  const toggleQuickSettings = useCallback(() => {
    setQuickSettingsOpen((prev) => !prev);
  }, []);

  const now = new Date();
  const weekday = now.toLocaleDateString("en-US", { weekday: "long" });
  const monthDay = now.toLocaleDateString("en-US", { month: "short", day: "numeric" });

  const fillWidth = Math.round((batteryLevel / 100) * 12);
  const batteryColor =
    batteryLevel <= 20 ? "#ea4335" : batteryLevel <= 50 ? "#fbbc04" : "white";

  return (
    <>
      <div className="fixed right-0 bottom-0 left-0 z-40 grid h-[56px] grid-cols-[auto_1fr_auto] items-center rounded-t-2xl bg-[rgba(0,0,0,0.65)] px-2 backdrop-blur-[60px] backdrop-saturate-[180%]">
        {/* Left: Launcher chip */}
        <button
          className="ml-1 flex cursor-pointer items-center gap-1.5 rounded-full bg-white/10 px-2.5 py-1.5 transition-colors hover:bg-white/15"
          onClick={toggleLauncher}
          aria-label="App launcher"
        >
          <div className="h-5 w-5 rounded-full border-2 border-white/60" />
        </button>

        {/* Center: Pinned apps */}
        <div className="flex items-center justify-center gap-[6px]">
          {pinnedApps.map((app) => (
            <motion.button
              key={app.name}
              className="flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center rounded-full transition-transform duration-150 hover:scale-110"
              whileTap={{
                scale: [1, 1.15, 0.95, 1],
                transition: { duration: 0.35 },
              }}
              aria-label={app.name}
            >
              <div
                className="flex h-[44px] w-[44px] items-center justify-center rounded-full"
                style={{ backgroundColor: app.color }}
              >
                {app.icon}
              </div>
            </motion.button>
          ))}
        </div>

        {/* Right: System tray */}
        <button
          className="mr-2 flex cursor-pointer items-center gap-2 rounded-full px-3 py-1.5 transition-colors hover:bg-white/10"
          onClick={toggleQuickSettings}
          aria-label="System tray"
        >
          <span className="text-[12px] font-medium text-white">
            {weekday}, {monthDay}&nbsp;&nbsp;{time || "--:--"}
          </span>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill={wifiEnabled ? "white" : "#9aa0a6"}
          >
            <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.24 4.24 0 00-6 0zm-4-4l2 2a7.07 7.07 0 0110 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
          </svg>
          <svg width="10" height="10" viewBox="0 0 24 24" fill="white">
            <path d="M7 10l5 5 5-5z" />
          </svg>
          <svg width="18" height="10" viewBox="0 0 20 12" fill="none">
            <rect
              x="0.75"
              y="0.75"
              width="15.5"
              height="10.5"
              rx="2"
              stroke="white"
              strokeWidth="1.5"
            />
            <rect
              x="2.5"
              y="2.5"
              width={fillWidth}
              height="7"
              rx="1"
              fill={batteryColor}
            />
            <path
              d="M17.5 4c.83 0 1.5.9 1.5 2s-.67 2-1.5 2"
              stroke="white"
              strokeWidth="1.5"
            />
          </svg>
        </button>
      </div>

      <AnimatePresence>
        {quickSettingsOpen && (
          <QuickSettings onClose={() => setQuickSettingsOpen(false)} />
        )}
      </AnimatePresence>
    </>
  );
}
