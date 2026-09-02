import { useState, useCallback, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import { useAliaStore } from "./stores/aliaStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { isNative } from "./lib/tauri";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Shelf from "./components/Shelf/Shelf";
import AppLauncher from "./components/AppLauncher/AppLauncher";
import OSD from "./components/Desktop/OSD";
import ScreenshotOverlay from "./components/Desktop/ScreenshotOverlay";
import AliaBubble from "./components/Alia/AliaBubble";
import AliaPanel from "./components/Alia/AliaPanel";
import NotificationToast from "./components/NotificationPanel/NotificationToast";
import NotificationPanel from "./components/NotificationPanel/NotificationPanel";
import SettingsPanel from "./components/Settings/SettingsPanel";
import QuickSettings from "./components/SystemTray/QuickSettings";
import CalendarPopup from "./components/Desktop/CalendarPopup";

/**
 * Single-page shell composition. Used by:
 *   • Web demo (no Tauri at all)
 *   • Native test mode "desktop" window — the shelf is rendered separately
 *     in its own always-on-top dock window, so we hide it here. Tray panels
 *     (launcher, quick settings, notifications, calendar) are rendered here
 *     and triggered by Tauri events fired from the shelf window.
 */
export default function WebApp() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  const isLocked = useSessionStore((s) => s.isLocked);
  const aliaOpen = useAliaStore((s) => s.isOpen);
  const [launcherOpen, setLauncherOpen] = useState(false);
  const [quickSettingsOpen, setQuickSettingsOpen] = useState(false);
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [calendarOpen, setCalendarOpen] = useState(false);
  useClock();
  useSystemInfo();

  const toggleLauncher = useCallback(() => setLauncherOpen((v) => !v), []);
  const closeLauncher = useCallback(() => setLauncherOpen(false), []);
  const openLauncher = useCallback(() => setLauncherOpen(true), []);
  const toggleQuickSettings = useCallback(() => setQuickSettingsOpen((v) => !v), []);
  const toggleNotifications = useCallback(() => setNotificationsOpen((v) => !v), []);
  const toggleCalendar = useCallback(() => setCalendarOpen((v) => !v), []);

  // Native test mode: the shelf is in its own window. Subscribe to Tauri events
  // it fires when its buttons are clicked, and toggle our local panel state.
  useEffect(() => {
    if (!isNative()) return;
    let disposed = false;
    let activeUnlisteners: (() => void)[] = [];

    const registration = import("@tauri-apps/api/event")
      .then(({ listen }) => Promise.all([
        listen("launcher-toggle", () => setLauncherOpen((v) => !v)),
        listen("launcher-show", () => setLauncherOpen(true)),
        listen("launcher-hide", () => setLauncherOpen(false)),
        listen("quicksettings-toggle", () => setQuickSettingsOpen((v) => !v)),
        listen("notifications-toggle", () => setNotificationsOpen((v) => !v)),
        listen("calendar-toggle", () => setCalendarOpen((v) => !v)),
      ]))
      .then((unlisteners) => {
        if (disposed) {
          for (const unlisten of unlisteners) unlisten();
        } else {
          activeUnlisteners = unlisteners;
        }
      })
      .catch((error: unknown) => {
        console.error("Failed to subscribe to native shell events", error);
      });

    return () => {
      disposed = true;
      for (const unlisten of activeUnlisteners) unlisten();
      void registration;
    };
  }, []);

  useKeyboardShortcuts({
    onToggleLauncher: toggleLauncher,
    onCloseLauncher: closeLauncher,
  });

  // In native test mode the shelf has its own window; don't double-render it here.
  const renderShelf = !isNative();

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

          <OSD />

          <AnimatePresence>
            {!aliaOpen && <AliaBubble key="alia-bubble" />}
          </AnimatePresence>
          <AnimatePresence>
            {aliaOpen && <AliaPanel key="alia-panel" />}
          </AnimatePresence>

          <ScreenshotOverlay />
          <NotificationToast />
          <SettingsPanel />

          {/* Tray panels: rendered here so they overflow the shelf bar's window */}
          <AnimatePresence>
            {quickSettingsOpen && (
              <QuickSettings key="qs" onClose={() => setQuickSettingsOpen(false)} />
            )}
          </AnimatePresence>
          <AnimatePresence>
            {notificationsOpen && (
              <NotificationPanel key="notif" onClose={() => setNotificationsOpen(false)} />
            )}
          </AnimatePresence>
          <AnimatePresence>
            {calendarOpen && (
              <CalendarPopup key="cal" onClose={() => setCalendarOpen(false)} />
            )}
          </AnimatePresence>

          {renderShelf && (
            <Shelf
              onToggleLauncher={openLauncher}
              onToggleQuickSettings={toggleQuickSettings}
              onToggleNotifications={toggleNotifications}
              onToggleCalendar={toggleCalendar}
            />
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
