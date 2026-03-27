import AppLauncher from "./AppLauncher";

/**
 * Root component for the launcher Tauri window.
 * The launcher runs in its own native window (fullscreen overlay, hidden by default).
 */
export default function LauncherWindow() {
  return (
    <div className="h-screen w-screen overflow-hidden bg-transparent">
      <AppLauncher />
    </div>
  );
}
