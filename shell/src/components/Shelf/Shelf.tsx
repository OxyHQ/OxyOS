import { useState, useCallback, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { useRunningAppsStore } from "../../stores/runningAppsStore";
import { invoke } from "../../lib/tauri";
import { launchApp } from "../../lib/launchApp";
import { getBatteryVisuals } from "../../lib/styles";
import { appExecMap } from "../../lib/appRegistry";
import QuickSettings from "../SystemTray/QuickSettings";
import NotificationPanel from "../NotificationPanel/NotificationPanel";
import CalendarPopup from "../Desktop/CalendarPopup";
import { useNotificationStore } from "../../stores/notificationStore";
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
  type TrayPanel = "quickSettings" | "notifications" | "calendar" | null;
  const [openPanel, setOpenPanel] = useState<TrayPanel>(null);
  const notificationCount = useNotificationStore((s) => s.notifications.length);
  const running = useRunningAppsStore((s) => s.running);
  const [bouncingApp, setBouncingApp] = useState<string | null>(null);
  const isLogin = variant === "login";

  const handleToggleLauncher = useCallback(() => {
    invoke("toggle_launcher");
  }, []);

  const togglePanel = useCallback((panel: TrayPanel) => {
    setOpenPanel((prev) => (prev === panel ? null : panel));
  }, []);

  const shortDate = useMemo(() => new Date().toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  }), [time]);

  const { fillWidth, fillColor } = getBatteryVisuals(batteryLevel, isCharging);

  return (
    <>
      {/* Full-width shelf */}
      <div className={`fixed right-0 bottom-0 left-0 grid h-[52px] items-center px-3 ${isLogin ? "grid-cols-[1fr_auto] bg-transparent" : "grid-cols-[1fr_auto_1fr] rounded-t-3xl border-t border-white/20 bg-white/12 shadow-[0_-4px_30px_rgba(0,0,0,0.2),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]"}`}>
        {!isLogin && (
          <>
            {/* Left: Launcher button */}
            <button
              className="flex h-[32px] w-[32px] cursor-pointer items-center justify-center justify-self-start rounded-full transition-colors duration-150 hover:bg-white/10"
              onClick={handleToggleLauncher}
              aria-label="App launcher"
              title="App Launcher"
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="white">
                <path d="M12 5C15.87 5 19 8.13 19 12C19 15.87 15.87 19 12 19C8.13 19 5 15.87 5 12C5 8.13 8.13 5 12 5M12 2C17.5 2 22 6.5 22 12C22 17.5 17.5 22 12 22C6.5 22 2 17.5 2 12C2 6.5 6.5 2 12 2M12 4C7.58 4 4 7.58 4 12C4 16.42 7.58 20 12 20C16.42 20 20 16.42 20 12C20 7.58 16.42 4 12 4Z" />
              </svg>
            </button>

            {/* Center: Pinned apps */}
            <div className="flex items-center justify-center gap-1">
              {pinnedApps.map((app) => (
                <div key={app.name} className="group relative flex flex-col items-center">
                  <motion.button
                    className="flex h-[40px] w-[40px] shrink-0 cursor-pointer items-center justify-center rounded-full transition-transform duration-150 hover:shadow-[0_2px_8px_rgba(255,255,255,0.2)] hover:brightness-110"
                    whileTap={{ scale: 0.92 }}
                    animate={bouncingApp === app.name ? { y: [0, -12, 0] } : { y: 0 }}
                    transition={bouncingApp === app.name ? { duration: 0.4, ease: "easeInOut" } : undefined}
                    onAnimationComplete={() => {
                      if (bouncingApp === app.name) setBouncingApp(null);
                    }}
                    onClick={() => {
                      const exec = appExecMap[app.name];
                      if (exec) {
                        launchApp(app.name, exec);
                        setBouncingApp(app.name);
                      }
                    }}
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
                  {/* Running indicator dot */}
                  <div
                    className={`absolute -bottom-1 left-1/2 h-[3px] w-[3px] -translate-x-1/2 rounded-full bg-white transition-opacity duration-200 ${running.has(app.name) ? "opacity-100" : "opacity-0"}`}
                  />
                  {/* Tooltip */}
                  <div className="pointer-events-none absolute -top-8 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-md bg-black/70 px-2 py-1 text-[10px] font-medium text-white opacity-0 backdrop-blur-sm transition-opacity duration-150 group-hover:opacity-100">
                    {app.name}
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {/* Spacer for login variant */}
        {isLogin && <div />}

        {/* Right: System tray */}
        <div className="flex h-[32px] items-center gap-[3px] justify-self-end">
          {/* Notifications button — desktop only */}
          {!isLogin && (
            <button
              className="relative flex h-[32px] w-[32px] cursor-pointer items-center justify-center rounded-full bg-white/10 transition-colors duration-150 hover:bg-white/15"
              onClick={() => togglePanel("notifications")}
              aria-label="Notifications"
              title="Notifications"
            >
              <svg viewBox="0 0 24 24" width="14" height="14" fill="white">
                <path d="M19.993 9.042C19.48 5.017 16.054 2 11.996 2s-7.49 3.021-7.999 7.051L2.866 18H7.1c.463 2.282 2.481 4 4.9 4s4.437-1.718 4.9-4h4.236l-1.143-8.958zM12 20c-1.306 0-2.417-.835-2.829-2h5.658c-.412 1.165-1.523 2-2.829 2zm-6.866-4l.847-6.698C6.364 6.272 8.941 4 11.996 4s5.627 2.268 6.013 5.295L18.864 16H5.134z" />
              </svg>
              {notificationCount > 0 && (
                <span className="absolute -top-0.5 -right-0.5 flex h-[14px] min-w-[14px] items-center justify-center rounded-full bg-[#ff453a] px-[3px] text-[9px] font-bold leading-none text-white">
                  {notificationCount > 9 ? "9+" : notificationCount}
                </span>
              )}
            </button>
          )}

          {/* Power button — login only */}
          {isLogin && (
            <button
              className="flex h-[32px] w-[32px] cursor-pointer items-center justify-center rounded-full bg-white/10 transition-colors duration-150 hover:bg-white/15"
              aria-label="Power"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
                <line x1="12" y1="2" x2="12" y2="12" />
              </svg>
            </button>
          )}

          {/* Icons pill — rounded left only */}
          <button
            className="flex h-[32px] cursor-pointer items-center gap-2 rounded-l-full bg-white/10 px-3 transition-colors duration-150 hover:bg-white/15"
            onClick={() => togglePanel("quickSettings")}
            aria-label="System status"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill={wifiEnabled ? "white" : "#9aa0a6"}>
              <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.24 4.24 0 00-6 0zm-4-4l2 2a7.07 7.07 0 0110 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
            </svg>
            {/* Battery icon */}
            <div className="relative flex items-center">
              <svg width="30" height="14" viewBox="0 0 30 14" fill="none" className="rotate-180">
                <rect x="0" y="0" width="25" height="14" rx="5" fill="white" opacity="0.25" />
                <rect x="0" y="0" width={fillWidth} height="14" rx="5" fill={fillColor} opacity="0.9" />
                <rect x="26" y="4" width="3" height="6" rx="1.5" fill="white" opacity="0.4" />
              </svg>
              {isCharging ? (
                <span className="absolute inset-0 flex items-center justify-center pl-1">
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
            onClick={() => togglePanel("calendar")}
            aria-label="Date and time"
          >
            <span className="text-[11px] font-medium text-white">{shortDate}&ensp;{time || "--:--"}</span>
          </button>
        </div>
      </div>

      <AnimatePresence>
        {openPanel === "quickSettings" && (
          <QuickSettings key="qs" onClose={() => setOpenPanel(null)} />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {openPanel === "notifications" && (
          <NotificationPanel key="notif" onClose={() => setOpenPanel(null)} />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {openPanel === "calendar" && (
          <CalendarPopup key="cal" onClose={() => setOpenPanel(null)} />
        )}
      </AnimatePresence>
    </>
  );
}
