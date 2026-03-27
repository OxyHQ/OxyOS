import { useCallback } from "react";
import TerminalApp from "./TerminalApp";

/**
 * Root component for a native Terminal Tauri window.
 * Provides its own custom title bar with drag region and window controls.
 */
export default function TerminalWindow({ windowId }: { windowId: string }) {
  const handleClose = useCallback(async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      getCurrentWindow().close();
    } catch {
      // Browser fallback
    }
  }, []);

  const handleStartDrag = useCallback(async (e: React.MouseEvent) => {
    // Only drag from the title bar area, not buttons
    if ((e.target as HTMLElement).closest("button")) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      getCurrentWindow().startDragging();
    } catch {
      // Browser fallback
    }
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden rounded-xl border border-white/15 bg-black/60 shadow-[0_12px_40px_rgba(0,0,0,0.5),inset_0_0.5px_0_rgba(255,255,255,0.08)] backdrop-blur-[60px] backdrop-saturate-[180%]">
      {/* Custom title bar */}
      <div
        className="flex h-[38px] shrink-0 cursor-default items-center justify-between px-3"
        onMouseDown={handleStartDrag}
      >
        {/* Traffic lights */}
        <div className="flex items-center gap-2">
          <button
            onClick={handleClose}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#ff5f57] transition-opacity hover:opacity-80"
            aria-label="Close"
          />
          <div className="h-3 w-3 rounded-full bg-[#febc2e] opacity-50" />
          <div className="h-3 w-3 rounded-full bg-[#28c840] opacity-50" />
        </div>

        {/* Title */}
        <span className="pointer-events-none text-[12px] font-medium text-white/50">
          Terminal
        </span>

        {/* Spacer */}
        <div className="w-[52px]" />
      </div>

      {/* Terminal content */}
      <div className="min-h-0 flex-1">
        <TerminalApp windowId={windowId} />
      </div>
    </div>
  );
}
