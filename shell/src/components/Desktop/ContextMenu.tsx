import { useState, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";

import { glass } from "../../lib/styles";

interface MenuPos {
  x: number;
  y: number;
}

export default function ContextMenu() {
  const [pos, setPos] = useState<MenuPos | null>(null);
  const setBrightness = useSystemStore((s) => s.setBrightness);

  const handleContext = useCallback((e: MouseEvent) => {
    const target = e.target as HTMLElement;
    // Only show on the wallpaper background itself
    if (
      target.closest("[data-desktop-bg]") &&
      !target.closest("button") &&
      !target.closest("[role]")
    ) {
      e.preventDefault();
      setPos({ x: e.clientX, y: e.clientY });
    }
  }, []);

  const handleClick = useCallback(() => setPos(null), []);

  useEffect(() => {
    window.addEventListener("contextmenu", handleContext);
    window.addEventListener("click", handleClick);
    return () => {
      window.removeEventListener("contextmenu", handleContext);
      window.removeEventListener("click", handleClick);
    };
  }, [handleContext, handleClick]);

  return (
    <AnimatePresence>
      {pos && (
        <motion.div
          initial={{ opacity: 0, scale: 0.92 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.95 }}
          transition={{ duration: 0.12 }}
          className={`${glass.menu} fixed z-[100] w-[200px] py-1.5`}
          style={{ left: pos.x, top: pos.y }}
        >
          <MenuItem label="Change Wallpaper" onClick={() => setPos(null)} />
          <MenuItem
            label="Toggle Night Light"
            onClick={() => {
              useSystemStore.getState().toggleNightLight();
              setPos(null);
            }}
          />
          <div className="mx-3 my-1 h-px bg-white/10" />
          <MenuItem label="Display Settings" onClick={() => setPos(null)} />
          <MenuItem
            label="Reset Brightness"
            onClick={() => {
              setBrightness(80);
              setPos(null);
            }}
          />
          <div className="mx-3 my-1 h-px bg-white/10" />
          <MenuItem label="About OxyOS" onClick={() => setPos(null)} />
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function MenuItem({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className="flex w-full cursor-pointer items-center px-3.5 py-1.5 text-left text-[13px] text-white/80 transition-colors duration-75 hover:bg-white/12"
    >
      {label}
    </button>
  );
}
