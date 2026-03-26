import { useCallback, useRef } from "react";
import { motion } from "framer-motion";
import { useWindowStore, type WindowState } from "../../stores/windowStore";

interface AppWindowProps {
  win: WindowState;
  children: React.ReactNode;
}

export default function AppWindow({ win, children }: AppWindowProps) {
  const moveWindow = useWindowStore((s) => s.moveWindow);
  const resizeWindow = useWindowStore((s) => s.resizeWindow);
  const focusWindow = useWindowStore((s) => s.focusWindow);
  const closeWindow = useWindowStore((s) => s.closeWindow);
  const toggleMaximize = useWindowStore((s) => s.toggleMaximize);
  const toggleMinimize = useWindowStore((s) => s.toggleMinimize);
  const dragRef = useRef<{ startX: number; startY: number; winX: number; winY: number } | null>(null);

  const handleTitlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (win.isMaximized) return;
      e.preventDefault();
      focusWindow(win.id);
      dragRef.current = { startX: e.clientX, startY: e.clientY, winX: win.x, winY: win.y };

      const handleMove = (ev: PointerEvent) => {
        if (!dragRef.current) return;
        const dx = ev.clientX - dragRef.current.startX;
        const dy = ev.clientY - dragRef.current.startY;
        moveWindow(win.id, dragRef.current.winX + dx, dragRef.current.winY + dy);
      };

      const handleUp = () => {
        dragRef.current = null;
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
    },
    [win.id, win.x, win.y, win.isMaximized, focusWindow, moveWindow],
  );

  // Resize from bottom-right corner
  const handleResizePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (win.isMaximized) return;
      e.preventDefault();
      e.stopPropagation();
      focusWindow(win.id);
      const startX = e.clientX;
      const startY = e.clientY;
      const startW = win.width;
      const startH = win.height;

      const handleMove = (ev: PointerEvent) => {
        const w = Math.max(400, startW + (ev.clientX - startX));
        const h = Math.max(280, startH + (ev.clientY - startY));
        resizeWindow(win.id, w, h);
      };

      const handleUp = () => {
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
    },
    [win.id, win.width, win.height, win.isMaximized, focusWindow, resizeWindow],
  );

  if (win.isMinimized) return null;

  const isMax = win.isMaximized;
  const style: React.CSSProperties = isMax
    ? { left: 0, top: 0, width: "100%", height: "calc(100% - 52px)", zIndex: win.zIndex }
    : { left: win.x, top: win.y, width: win.width, height: win.height, zIndex: win.zIndex };

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.95 }}
      transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
      className={`absolute flex flex-col overflow-hidden border border-white/15 bg-black/60 shadow-[0_12px_40px_rgba(0,0,0,0.5),inset_0_0.5px_0_rgba(255,255,255,0.08)] backdrop-blur-[60px] backdrop-saturate-[180%] ${isMax ? "rounded-none" : "rounded-xl"}`}
      style={style}
      onPointerDown={() => focusWindow(win.id)}
    >
      {/* Title bar */}
      <div
        className="flex h-[38px] shrink-0 cursor-default items-center justify-between px-3"
        onPointerDown={handleTitlePointerDown}
        onDoubleClick={() => toggleMaximize(win.id)}
      >
        {/* Traffic lights */}
        <div className="flex items-center gap-2">
          <button
            onClick={(e) => { e.stopPropagation(); closeWindow(win.id); }}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#ff5f57] transition-opacity hover:opacity-80"
            aria-label="Close"
          />
          <button
            onClick={(e) => { e.stopPropagation(); toggleMinimize(win.id); }}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#febc2e] transition-opacity hover:opacity-80"
            aria-label="Minimize"
          />
          <button
            onClick={(e) => { e.stopPropagation(); toggleMaximize(win.id); }}
            className="h-3 w-3 cursor-pointer rounded-full bg-[#28c840] transition-opacity hover:opacity-80"
            aria-label="Maximize"
          />
        </div>

        {/* Title */}
        <span className="pointer-events-none text-[12px] font-medium text-white/50">
          {win.title}
        </span>

        {/* Spacer */}
        <div className="w-[52px]" />
      </div>

      {/* Content */}
      <div className="min-h-0 flex-1">
        {children}
      </div>

      {/* Resize handle (bottom-right) */}
      {!isMax && (
        <div
          className="absolute right-0 bottom-0 h-4 w-4 cursor-nwse-resize"
          onPointerDown={handleResizePointerDown}
        />
      )}
    </motion.div>
  );
}
