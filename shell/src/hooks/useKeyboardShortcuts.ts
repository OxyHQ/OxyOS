import { useEffect } from "react";
import { invoke } from "../lib/tauri";
import { useScreenshotStore } from "../stores/screenshotStore";

export function useKeyboardShortcuts() {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Super/Meta key to toggle launcher
      if (e.key === "Meta" && !e.repeat) {
        e.preventDefault();
        invoke("toggle_launcher");
      }

      // Ctrl+Shift+S to activate screenshot tool
      if (e.ctrlKey && e.shiftKey && e.key === "S") {
        e.preventDefault();
        useScreenshotStore.getState().activate();
      }

      // Escape to close launcher
      if (e.key === "Escape") {
        invoke("hide_launcher");
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);
}
