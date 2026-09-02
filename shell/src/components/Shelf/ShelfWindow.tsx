import { useCallback } from "react";
import { invoke } from "../../lib/tauri";
import Shelf from "./Shelf";

/**
 * Shelf in its own opaque always-on-top dock window.
 * Solid #202124 — exact ChromeOS Ash dark-mode shelf color.
 * Panels (launcher, quick settings, notifications, calendar) cannot render
 * here because they need to overflow the 52px shelf window — instead we
 * fire Tauri events that the desktop window listens for and renders.
 */
export default function ShelfWindow() {
  const onToggleLauncher = useCallback(() => { invoke("toggle_launcher"); }, []);
  const onToggleQuickSettings = useCallback(() => { invoke("toggle_quicksettings"); }, []);
  const onToggleNotifications = useCallback(() => { invoke("toggle_notifications"); }, []);
  const onToggleCalendar = useCallback(() => { invoke("toggle_calendar"); }, []);

  return (
    <div className="h-screen w-screen overflow-hidden" style={{ background: "#202124" }}>
      <Shelf
        chrome="dock"
        onToggleLauncher={onToggleLauncher}
        onToggleQuickSettings={onToggleQuickSettings}
        onToggleNotifications={onToggleNotifications}
        onToggleCalendar={onToggleCalendar}
      />
    </div>
  );
}
