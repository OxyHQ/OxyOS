import { useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";
import { useSystemStore } from "../../stores/systemStore";
import QuickSettings from "../SystemTray/QuickSettings";
import browserIcon from "../../assets/icons/browser.svg";
import filesIcon from "../../assets/icons/files.svg";
import settingsIcon from "../../assets/icons/settings.svg";
import terminalIcon from "../../assets/icons/terminal.svg";
import storeIcon from "../../assets/icons/store.svg";

interface PinnedApp {
  name: string;
  icon: string;
}

const pinnedApps: PinnedApp[] = [
  { name: "Browser", icon: browserIcon },
  { name: "Files", icon: filesIcon },
  { name: "Settings", icon: settingsIcon },
  { name: "Terminal", icon: terminalIcon },
  { name: "Store", icon: storeIcon },
];

interface ShelfProps {
  variant?: "desktop" | "login";
}

export default function Shelf({ variant = "desktop" }: ShelfProps) {
  const time = useSystemStore((s) => s.time);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const isCharging = useSystemStore((s) => s.isCharging);
  const [quickSettingsOpen, setQuickSettingsOpen] = useState(false);
  const toggleLauncher = useLauncherStore((s) => s.toggle);
  const isLogin = variant === "login";

  const toggleQuickSettings = useCallback(() => {
    setQuickSettingsOpen((prev) => !prev);
  }, []);

  const shortDate = new Date().toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });

  const fillWidth = Math.round((batteryLevel / 100) * 18);
  const fillColor = isCharging ? "#30d158" : batteryLevel <= 20 ? "#ff453a" : "white";

  return (
    <>
      {/* Full-width shelf — rounded top, blurred backdrop */}
      <div className={`fixed right-0 bottom-0 left-0 z-40 grid h-[52px] items-center px-3 ${isLogin ? "grid-cols-[1fr_auto] bg-transparent" : "grid-cols-[1fr_auto_1fr] rounded-t-3xl bg-black/50 backdrop-blur-[60px] backdrop-saturate-[180%]"}`}>
        {!isLogin && (
          <>
            {/* Left: Launcher button */}
            <button
              className="flex h-[32px] w-[32px] cursor-pointer items-center justify-center justify-self-start rounded-full transition-colors duration-150 hover:bg-white/10"
              onClick={toggleLauncher}
              aria-label="App launcher"
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="white">
                <path d="M12 5C15.87 5 19 8.13 19 12C19 15.87 15.87 19 12 19C8.13 19 5 15.87 5 12C5 8.13 8.13 5 12 5M12 2C17.5 2 22 6.5 22 12C22 17.5 17.5 22 12 22C6.5 22 2 17.5 2 12C2 6.5 6.5 2 12 2M12 4C7.58 4 4 7.58 4 12C4 16.42 7.58 20 12 20C16.42 20 20 16.42 20 12C20 7.58 16.42 4 12 4Z" />
              </svg>
            </button>

            {/* Center: Pinned apps */}
            <div className="flex items-center justify-center gap-1">
              {pinnedApps.map((app) => (
                <motion.button
                  key={app.name}
                  className="flex h-[40px] w-[40px] shrink-0 cursor-pointer items-center justify-center rounded-full transition-transform duration-150 hover:shadow-[0_2px_8px_rgba(255,255,255,0.2)] hover:brightness-110"
                  whileTap={{ scale: 0.92 }}
                  aria-label={app.name}
                >
                  <div className="h-[36px] w-[36px] overflow-hidden rounded-full">
                    <img
                      src={app.icon}
                      alt={app.name}
                      className="h-full w-full object-cover"
                      draggable={false}
                    />
                  </div>
                </motion.button>
              ))}
            </div>
          </>
        )}

        {/* Spacer for login variant to push system tray right */}
        {isLogin && <div />}

        {/* Right: Power + System tray */}
        <div className="flex h-[32px] items-center gap-[3px] justify-self-end">
          {/* Notifications button — desktop only */}
          {!isLogin && (
            <button
              className="flex h-[32px] w-[32px] cursor-pointer items-center justify-center rounded-full bg-white/10 transition-colors duration-150 hover:bg-white/15"
              aria-label="Notifications"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="white">
                <path d="M12 22c1.1 0 2-.9 2-2h-4a2 2 0 0 0 2 2zm6-6v-5c0-3.07-1.63-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.64 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z" />
              </svg>
            </button>
          )}

          {/* Power button */}
          <button
            className="flex h-[32px] w-[32px] cursor-pointer items-center justify-center rounded-full bg-white/10 transition-colors duration-150 hover:bg-white/15"
            aria-label="Power"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
              <line x1="12" y1="2" x2="12" y2="12" />
            </svg>
          </button>

          {/* Icons pill — rounded left only */}
          <button
            className="flex h-[32px] cursor-pointer items-center gap-2 rounded-l-full bg-white/10 px-3 transition-colors duration-150 hover:bg-white/15"
            onClick={toggleQuickSettings}
            aria-label="System status"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill={wifiEnabled ? "white" : "#9aa0a6"}>
              <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.24 4.24 0 00-6 0zm-4-4l2 2a7.07 7.07 0 0110 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
            </svg>
            {/* Battery icon — themed fill + lightning bolt when charging */}
            <div className="relative flex items-center">
              <svg width="30" height="14" viewBox="0 0 30 14" fill="none" className="rotate-180">
                <rect x="0" y="0" width="25" height="14" rx="5" fill="white" opacity="0.25" />
                <rect x="0" y="0" width={fillWidth} height="14" rx="5" fill={fillColor} opacity="0.9" />
                <rect x="26" y="4" width="3" height="6" rx="1.5" fill="white" opacity="0.4" />
              </svg>
              {isCharging ? (
                <span className="absolute inset-0 flex items-center justify-center pl-1">
                  {/* Lightning bolt */}
                  <svg width="10" height="12" viewBox="0 0 12 16" fill="none" className="rotate-180">
                    <path d="M7 0L3 9h3l-1 7 5-10H7V0z" fill="white" />
                  </svg>
                </span>
              ) : (
                <span className="absolute inset-0 flex items-center justify-center pl-1 text-[8px] font-bold leading-none text-white mix-blend-difference">
                  {batteryLevel}
                </span>
              )}
            </div>
          </button>

          {/* Date + time pill — rounded right only */}
          <button
            className="flex h-[32px] cursor-pointer items-center rounded-r-full bg-white/10 px-3 transition-colors duration-150 hover:bg-white/15"
            onClick={toggleQuickSettings}
            aria-label="Date and time"
          >
            <span className="text-[11px] font-medium text-white">{shortDate}&ensp;{time || "--:--"}</span>
          </button>

        </div>
      </div>

      <AnimatePresence>
        {quickSettingsOpen && (
          <QuickSettings onClose={() => setQuickSettingsOpen(false)} />
        )}
      </AnimatePresence>
    </>
  );
}
