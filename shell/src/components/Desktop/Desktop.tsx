import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";
import { useAliaStore } from "../../stores/aliaStore";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";
import Shelf from "../Shelf/Shelf";
import AppLauncher from "../AppLauncher/AppLauncher";
import ContextMenu from "./ContextMenu";
import OSD from "./OSD";
import AliaBubble from "../Alia/AliaBubble";
import AliaPanel from "../Alia/AliaPanel";

export default function Desktop() {
  const isLauncherOpen = useLauncherStore((s) => s.isOpen);
  const closeLauncher = useLauncherStore((s) => s.close);
  const aliaOpen = useAliaStore((s) => s.isOpen);
  useKeyboardShortcuts();

  return (
    <motion.div
      data-desktop-bg
      className="wallpaper-bg relative h-screen w-screen select-none overflow-hidden"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.5 }}
    >
      {/* Scrim overlay when launcher is open */}
      <AnimatePresence>
        {isLauncherOpen && (
          <motion.div
            className="absolute inset-0 z-30 bg-black/40"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={closeLauncher}
          />
        )}
      </AnimatePresence>

      {/* App Launcher */}
      <AppLauncher />

      {/* Desktop right-click menu */}
      <ContextMenu />

      {/* Volume/brightness OSD */}
      <OSD />

      {/* Alia AI assistant */}
      <AnimatePresence>
        {!aliaOpen && <AliaBubble key="alia-bubble" />}
      </AnimatePresence>
      <AnimatePresence>
        {aliaOpen && <AliaPanel key="alia-panel" />}
      </AnimatePresence>

      {/* Shelf */}
      <Shelf />
    </motion.div>
  );
}
