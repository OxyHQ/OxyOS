import { useState } from "react";
import { AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Desktop from "./components/Desktop/Desktop";
import ShelfWindow from "./components/Shelf/ShelfWindow";
import LauncherWindow from "./components/AppLauncher/LauncherWindow";
import TerminalWindow from "./components/Terminal/TerminalWindow";

function getWindowLabel(): string {
  try {
    if (window.__TAURI_INTERNALS__) {
      // In Tauri 2, the current window label is available on the internals
      const metadata = window.__TAURI_INTERNALS__?.metadata;
      if (metadata?.currentWebview?.label) {
        return metadata.currentWebview.label;
      }
    }
  } catch {
    // Browser mode fallback
  }
  return "desktop";
}

function DesktopApp() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  const isLocked = useSessionStore((s) => s.isLocked);
  useClock();
  useSystemInfo();

  const showLogin = !isLoggedIn || isLocked;

  return (
    <AnimatePresence mode="wait">
      {showLogin ? (
        <LoginScreen key="login" />
      ) : (
        <Desktop key="desktop" />
      )}
    </AnimatePresence>
  );
}

export default function App() {
  const [windowLabel] = useState(getWindowLabel);

  // All windows need clock and system info
  useClock();
  useSystemInfo();

  if (windowLabel === "shelf") {
    return <ShelfWindow />;
  }

  if (windowLabel === "launcher") {
    return <LauncherWindow />;
  }

  if (windowLabel.startsWith("terminal-")) {
    return <TerminalWindow windowId={windowLabel} />;
  }

  // Default: desktop window
  return <DesktopApp />;
}
