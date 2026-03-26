import { useEffect } from "react";
import { useLauncherStore } from "../stores/launcherStore";
import { useScreenshotStore } from "../stores/screenshotStore";

export function useKeyboardShortcuts() {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Super/Meta key to toggle launcher
      if (e.key === "Meta" && !e.repeat) {
        e.preventDefault();
        useLauncherStore.getState().toggle();
      }

      // Ctrl+Shift+S to activate screenshot tool
      if (e.ctrlKey && e.shiftKey && e.key === "S") {
        e.preventDefault();
        useScreenshotStore.getState().activate();
      }

      // Escape to close launcher and any open panels
      if (e.key === "Escape") {
        useLauncherStore.getState().close();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);
}
