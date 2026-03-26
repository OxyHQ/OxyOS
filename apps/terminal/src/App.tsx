import { useCallback } from "react";
import TerminalView from "./TerminalView";

export default function App() {
  const handleClose = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().close();
      });
    }
  }, []);

  const handleMinimize = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().minimize();
      });
    }
  }, []);

  const handleMaximize = useCallback(() => {
    if (window.__TAURI_INTERNALS__) {
      import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
        getCurrentWindow().toggleMaximize();
      });
    }
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden rounded-xl border border-white/15 bg-black/60 shadow-[0_12px_40px_rgba(0,0,0,0.5),inset_0_0.5px_0_rgba(255,255,255,0.08)] backdrop-blur-[60px] backdrop-saturate-[180%]">
      {/* Custom title bar */}
      <div
        data-tauri-drag-region
        className="flex h-[38px] shrink-0 items-center justify-between px-3"
      >
        {/* Traffic light buttons */}
        <div className="flex items-center gap-2">
          <button
            onClick={handleClose}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#ff5f57] transition-opacity hover:opacity-80"
            aria-label="Close"
          />
          <button
            onClick={handleMinimize}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#febc2e] transition-opacity hover:opacity-80"
            aria-label="Minimize"
          />
          <button
            onClick={handleMaximize}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#28c840] transition-opacity hover:opacity-80"
            aria-label="Maximize"
          />
        </div>

        {/* Title */}
        <span className="pointer-events-none text-[12px] font-medium text-white/50">
          Terminal
        </span>

        {/* Spacer to center title */}
        <div className="w-[52px]" />
      </div>

      {/* Terminal content */}
      <div className="min-h-0 flex-1">
        <TerminalView />
      </div>
    </div>
  );
}
