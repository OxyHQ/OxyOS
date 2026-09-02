import { lazy, Suspense, useState } from "react";
import { AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import { isNative } from "./lib/tauri";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Desktop from "./components/Desktop/Desktop";
import ShelfWindow from "./components/Shelf/ShelfWindow";
import LauncherWindow from "./components/AppLauncher/LauncherWindow";
import WebApp from "./WebApp";
import QuickSettings from "./components/SystemTray/QuickSettings";
import NotificationPanel from "./components/NotificationPanel/NotificationPanel";
import CalendarPopup from "./components/Desktop/CalendarPopup";
import { invoke } from "./lib/tauri";

const TerminalWindow = lazy(() => import("./components/Terminal/TerminalWindow"));

function getWindowLabel(): string | null {
  return window.__TAURI_INTERNALS__?.metadata?.currentWebview?.label ?? null;
}

function DesktopApp() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  const isLocked = useSessionStore((s) => s.isLocked);
  const showLogin = !isLoggedIn || isLocked;

  return (
    <AnimatePresence mode="wait">
      {showLogin ? <LoginScreen key="login" /> : <Desktop key="desktop" />}
    </AnimatePresence>
  );
}

function NativeRoot({ windowLabel }: { windowLabel: string }) {
  useClock();
  useSystemInfo();

  // Test mode: a single "desktop" window exists. Render the same
  // single-page layout the web demo uses — shelf, launcher overlay,
  // login, settings, etc. all inside one React tree, no inter-window flicker.
  if (windowLabel === "desktop") {
    return <WebApp />;
  }

  // Production mode windows (only spawned when running as actual session shell)
  if (windowLabel === "shelf") return <ShelfWindow />;
  if (windowLabel === "launcher") return <LauncherWindow />;
  if (windowLabel === "quicksettings") {
    return (
      <div className="h-screen w-screen overflow-hidden bg-transparent">
        <QuickSettings onClose={() => invoke("toggle_quicksettings")} />
      </div>
    );
  }
  if (windowLabel === "notifications") {
    return (
      <div className="h-screen w-screen overflow-hidden bg-transparent">
        <NotificationPanel onClose={() => invoke("toggle_notifications")} />
      </div>
    );
  }
  if (windowLabel === "calendar") {
    return (
      <div className="h-screen w-screen overflow-hidden bg-transparent">
        <CalendarPopup onClose={() => invoke("toggle_calendar")} />
      </div>
    );
  }
  if (windowLabel.startsWith("terminal-")) {
    return (
      <Suspense fallback={<div className="h-screen w-screen bg-[#111827]" />}>
        <TerminalWindow windowId={windowLabel} />
      </Suspense>
    );
  }
  return <DesktopApp />;
}

export default function App() {
  const [windowLabel] = useState(getWindowLabel);

  if (!isNative() || windowLabel === null) {
    return <WebApp />;
  }
  return <NativeRoot windowLabel={windowLabel} />;
}
