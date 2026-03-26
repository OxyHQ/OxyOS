import { motion } from "framer-motion";
import { useAliaStore } from "../../stores/aliaStore";
import AliaFace from "./AliaFace";
import type { AliaExpression } from "./AliaFace";
import { oxGlassPresets } from "../../lib/styles";
import OxGlass from "../shared/OxGlass";

export default function AliaBubble() {
  const isOpen = useAliaStore((s) => s.isOpen);
  const isListening = useAliaStore((s) => s.isListening);
  const isStreaming = useAliaStore((s) => s.isStreaming);
  const toggle = useAliaStore((s) => s.toggle);

  if (isOpen) return null;

  const expression: AliaExpression = isListening
    ? "Interesting"
    : isStreaming
      ? "Writing E"
      : "Idle A";

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      transition={{ duration: 0.2, ease: [0.2, 0, 0, 1] }}
      className="fixed top-4 right-4 z-50"
    >
      <OxGlass
        {...oxGlassPresets.pill}
        className="relative flex h-11 w-11 cursor-pointer items-center justify-center rounded-full border border-white/20 shadow-[0_4px_20px_rgba(0,0,0,0.3),inset_0_0.5px_0_rgba(255,255,255,0.12)] transition-transform duration-150 hover:scale-110"
      >
        <button
          onClick={toggle}
          className="flex h-full w-full cursor-pointer items-center justify-center rounded-full"
          aria-label="Open Alia"
        >
          {isListening && (
            <span className="absolute inset-0 animate-ping rounded-full bg-[#0a84ff]/30" />
          )}
          {isStreaming && (
            <span className="absolute inset-[-2px] animate-spin rounded-full border-2 border-transparent border-t-[#0a84ff]/50" style={{ animationDuration: "1.5s" }} />
          )}
          <AliaFace size={28} expression={expression} />
        </button>
      </OxGlass>
    </motion.div>
  );
}
