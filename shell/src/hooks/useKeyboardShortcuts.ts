import { useEffect } from "react";
import { useLauncherStore } from "../stores/launcherStore";

export function useKeyboardShortcuts() {
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Super/Meta key to toggle launcher
      if (e.key === "Meta" && !e.repeat) {
        e.preventDefault();
        useLauncherStore.getState().toggle();
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
