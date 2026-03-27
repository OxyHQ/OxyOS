import { useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import { useAliaStore } from "./stores/aliaStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Shelf from "./components/Shelf/Shelf";
import AppLauncher from "./components/AppLauncher/AppLauncher";
import OSD from "./components/Desktop/OSD";
import ScreenshotOverlay from "./components/Desktop/ScreenshotOverlay";
import AliaBubble from "./components/Alia/AliaBubble";
import AliaPanel from "./components/Alia/AliaPanel";
import NotificationToast from "./components/NotificationPanel/NotificationToast";
import SettingsPanel from "./components/Settings/SettingsPanel";

/**
 * Web demo mode — single-page shell with all components composed together.
 * Used when running in a browser without Tauri (e.g. os.oxy.so/demo).
 */
export default function WebApp() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  const isLocked = useSessionStore((s) => s.isLocked);
  const aliaOpen = useAliaStore((s) => s.isOpen);
  const [launcherOpen, setLauncherOpen] = useState(false);
  useClock();
  useSystemInfo();

  const toggleLauncher = useCallback(() => {
    setLauncherOpen((prev) => !prev);
  }, []);

  const closeLauncher = useCallback(() => {
    setLauncherOpen(false);
  }, []);

  useKeyboardShortcuts({
    onToggleLauncher: toggleLauncher,
    onCloseLauncher: closeLauncher,
  });

  const showLogin = !isLoggedIn || isLocked;

  return (
    <AnimatePresence mode="wait">
      {showLogin ? (
        <LoginScreen key="login" />
      ) : (
        <motion.div
          key="desktop"
          data-desktop-bg
          className="wallpaper-bg relative h-screen w-screen select-none overflow-hidden"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.5 }}
        >
          {/* Scrim overlay when launcher is open */}
          <AnimatePresence>
            {launcherOpen && (
              <motion.div
                className="absolute inset-0 z-[1100] bg-black/40"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2 }}
                onClick={closeLauncher}
              />
            )}
          </AnimatePresence>

          {/* App Launcher */}
          <AnimatePresence>
            {launcherOpen && (
              <motion.div
                key="launcher"
                className="fixed inset-0 z-[1200]"
                initial={{ opacity: 0, scale: 0.92 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.95 }}
                transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
              >
                <AppLauncher onClose={closeLauncher} />
              </motion.div>
            )}
          </AnimatePresence>

          {/* Volume/brightness OSD */}
          <OSD />

          {/* Alia AI assistant */}
          <AnimatePresence>
            {!aliaOpen && <AliaBubble key="alia-bubble" />}
          </AnimatePresence>
          <AnimatePresence>
            {aliaOpen && <AliaPanel key="alia-panel" />}
          </AnimatePresence>

          {/* Screenshot tool */}
          <ScreenshotOverlay />

          {/* Notification toasts */}
          <NotificationToast />

          {/* Settings panel */}
          <SettingsPanel />

          {/* Shelf */}
          <Shelf onToggleLauncher={toggleLauncher} />
        </motion.div>
      )}
    </AnimatePresence>
  );
}
