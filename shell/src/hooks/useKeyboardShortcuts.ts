import { useEffect } from "react";
import { useScreenshotStore } from "../stores/screenshotStore";

interface KeyboardShortcutOptions {
  onToggleLauncher?: () => void;
  onCloseLauncher?: () => void;
}

export function useKeyboardShortcuts(options?: KeyboardShortcutOptions) {
  const { onToggleLauncher, onCloseLauncher } = options ?? {};

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Meta" && !e.repeat) {
        e.preventDefault();
        onToggleLauncher?.();
      }
      if (e.ctrlKey && e.shiftKey && e.key === "S") {
        e.preventDefault();
        useScreenshotStore.getState().activate();
      }
      if (e.key === "Escape") {
        onCloseLauncher?.();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onToggleLauncher, onCloseLauncher]);
}
