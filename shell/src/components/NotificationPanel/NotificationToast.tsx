import { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useNotificationStore, type Notification } from "../../stores/notificationStore";
import { useOxGlass } from "../../hooks/useOxGlass";
import OxGlassFilter from "../shared/OxGlassFilter";
import GradientBlur from "../shared/GradientBlur";
import { oxGlassPresets } from "../../lib/styles";
import { appIcons, settingsIcon } from "./appIcons";

const AUTO_DISMISS_MS = 4000;

export default function NotificationToast() {
  const notifications = useNotificationStore((s) => s.notifications);
  const dismiss = useNotificationStore((s) => s.dismiss);

  const [visible, setVisible] = useState<Notification | null>(null);
  const prevCountRef = useRef(notifications.length);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const prevCount = prevCountRef.current;
    prevCountRef.current = notifications.length;

    // A new notification was added (prepended to the front of the array)
    if (notifications.length > prevCount && notifications.length > 0) {
      const latest = notifications[0];

      // Clear any existing auto-dismiss timer
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }

      setVisible(latest);

      timerRef.current = setTimeout(() => {
        setVisible(null);
        timerRef.current = null;
      }, AUTO_DISMISS_MS);
    }
  }, [notifications]);

  // Cleanup timer on unmount
  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
    };
  }, []);

  const oxglass = useOxGlass(oxGlassPresets.toast);

  function handleDismiss() {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    if (visible) {
      dismiss(visible.id);
    }
    setVisible(null);
  }

  return (
    <>
    {oxglass.filterData && (
      <OxGlassFilter
        filterId={oxglass.filterId}
        filterData={oxglass.filterData}
        blur={oxglass.blur}
        specularOpacity={oxglass.specularOpacity}
        specularSaturation={oxglass.specularSaturation}
      />
    )}
    <AnimatePresence>
      {visible && (
        <motion.div
          key={visible.id}
          initial={{ y: -60, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: -60, opacity: 0 }}
          transition={{ duration: 0.3, ease: [0.2, 0, 0, 1] }}
          className="fixed top-3 left-1/2 z-[100] w-[380px] -translate-x-1/2"
        >
        <div ref={oxglass.ref} className="rounded-[18px] border border-white/15 shadow-[0_8px_32px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.1)] px-4 py-3" style={{ ...oxglass.glassStyle, overflow: "hidden", position: "relative" }}>
          <GradientBlur direction="top" size={20} />
          <GradientBlur direction="bottom" size={20} />
          <div className="flex items-start gap-3">
            {/* App icon */}
            <img
              src={appIcons[visible.app] ?? settingsIcon}
              alt={visible.app}
              className="mt-0.5 h-8 w-8 shrink-0 rounded-lg object-cover"
              draggable={false}
            />

            {/* Content */}
            <div className="min-w-0 flex-1">
              <p className="truncate text-[13px] font-semibold leading-tight text-white/90">
                {visible.title}
              </p>
              <p className="mt-0.5 truncate text-[12px] leading-snug text-white/55">
                {visible.body}
              </p>
            </div>

            {/* Dismiss button */}
            <button
              onClick={handleDismiss}
              className="flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded-full bg-white/10 text-white/40 transition-colors duration-150 hover:bg-white/20 hover:text-white/70"
              aria-label="Dismiss notification"
            >
              <svg
                width="8"
                height="8"
                viewBox="0 0 10 10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              >
                <path d="M1 1l8 8M9 1l-8 8" />
              </svg>
            </button>
          </div>
        </div>
        </motion.div>
      )}
    </AnimatePresence>
    </>
  );
}
