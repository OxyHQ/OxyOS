import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";
import Shelf from "../Shelf/Shelf";
import AppLauncher from "../AppLauncher/AppLauncher";

export default function Desktop() {
  const isLauncherOpen = useLauncherStore((s) => s.isOpen);
  const closeLauncher = useLauncherStore((s) => s.close);

  return (
    <motion.div
      className="wallpaper-bg relative h-screen w-screen overflow-hidden"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.5 }}
    >
      {/* Scrim overlay when launcher is open */}
      <AnimatePresence>
        {isLauncherOpen && (
          <motion.div
            className="absolute inset-0 z-30"
            style={{ backgroundColor: "rgba(0, 0, 0, 0.4)" }}
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

      {/* Shelf (taskbar) */}
      <Shelf />
    </motion.div>
  );
}
