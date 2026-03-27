import { motion, AnimatePresence } from "framer-motion";
import { useAliaStore } from "../../stores/aliaStore";
import { useKeyboardShortcuts } from "../../hooks/useKeyboardShortcuts";

import OSD from "./OSD";
import ScreenshotOverlay from "./ScreenshotOverlay";
import AliaBubble from "../Alia/AliaBubble";
import AliaPanel from "../Alia/AliaPanel";
import NotificationToast from "../NotificationPanel/NotificationToast";
import SettingsPanel from "../Settings/SettingsPanel";

export default function Desktop() {
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
      {/* Volume/brightness OSD */}
      <OSD />

      {/* Alia AI assistant */}
      <AnimatePresence>
        {!aliaOpen && <AliaBubble key="alia-bubble" />}
      </AnimatePresence>
      <AnimatePresence>
        {aliaOpen && <AliaPanel key="alia-panel" />}
      </AnimatePresence>

      {/* Screenshot tool */}
      <ScreenshotOverlay />

      {/* Notification toasts */}
      <NotificationToast />

      {/* Settings panel */}
      <SettingsPanel />
    </motion.div>
  );
}
