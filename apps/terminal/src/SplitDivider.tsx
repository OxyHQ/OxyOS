import { useCallback, useRef } from "react";

interface SplitDividerProps {
  direction: "h" | "v";
  onResize: (ratio: number) => void;
}

export default function SplitDivider({ direction, onResize }: SplitDividerProps) {
  const dividerRef = useRef<HTMLDivElement>(null);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const divider = dividerRef.current;
      if (!divider) return;

      const parent = divider.parentElement;
      if (!parent) return;

      const rect = parent.getBoundingClientRect();
      divider.setPointerCapture(e.pointerId);

      const handleMove = (ev: PointerEvent) => {
        let ratio: number;
        if (direction === "h") {
          ratio = (ev.clientX - rect.left) / rect.width;
        } else {
          ratio = (ev.clientY - rect.top) / rect.height;
        }
        onResize(ratio);
      };

      const handleUp = () => {
        divider.removeEventListener("pointermove", handleMove);
        divider.removeEventListener("pointerup", handleUp);
      };

      divider.addEventListener("pointermove", handleMove);
      divider.addEventListener("pointerup", handleUp);
    },
    [direction, onResize],
  );

  return (
    <div
      ref={dividerRef}
      onPointerDown={handlePointerDown}
      className={`shrink-0 bg-white/8 transition-colors hover:bg-[#0a84ff]/50 ${
        direction === "h"
          ? "w-[4px] cursor-col-resize"
          : "h-[4px] cursor-row-resize"
      }`}
    />
  );
}
