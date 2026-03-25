import { useState, useCallback } from "react";
import { motion, useAnimation } from "framer-motion";
import { useSessionStore } from "../../stores/sessionStore";
import { useSystemStore } from "../../stores/systemStore";

const CORRECT_PIN = "1234";
const MAX_PIN_LENGTH = 6;

function WifiIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M5 12.55a11 11 0 0 1 14 0" />
      <path d="M8.53 16.11a6 6 0 0 1 6.95 0" />
      <circle cx="12" cy="20" r="1" fill="currentColor" stroke="none" />
    </svg>
  );
}

function BatteryIcon({ level }: { level: number }) {
  const fillWidth = Math.round((level / 100) * 14);
  return (
    <svg width="24" height="14" viewBox="0 0 24 14" fill="none">
      <rect
        x="0.5"
        y="0.5"
        width="20"
        height="13"
        rx="2"
        stroke="currentColor"
        strokeWidth="1"
      />
      <rect
        x="2"
        y="2.5"
        width={fillWidth}
        height="9"
        rx="1"
        fill="currentColor"
      />
      <rect x="21" y="4" width="2.5" height="6" rx="1" fill="currentColor" />
    </svg>
  );
}

function AvatarPlaceholder() {
  return (
    <div className="flex items-center justify-center w-20 h-20 rounded-full bg-gray-500/60">
      <svg
        width="40"
        height="40"
        viewBox="0 0 24 24"
        fill="rgba(255,255,255,0.8)"
      >
        <circle cx="12" cy="8" r="4" />
        <path d="M4 20c0-4 4-7 8-7s8 3 8 7" />
      </svg>
    </div>
  );
}

export default function LoginScreen() {
  const [pin, setPin] = useState("");
  const username = useSessionStore((s) => s.username);
  const time = useSystemStore((s) => s.time);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const shakeControls = useAnimation();

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (pin === CORRECT_PIN) {
        useSessionStore.getState().login();
      } else {
        shakeControls.start({
          x: [0, -12, 12, -8, 8, -4, 4, 0],
          transition: { duration: 0.4 },
        });
        setPin("");
      }
    },
    [pin, shakeControls],
  );

  const handlePinChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value.replace(/\D/g, "");
      if (value.length <= MAX_PIN_LENGTH) {
        setPin(value);
      }
    },
    [],
  );

  return (
    <motion.div
      className="wallpaper-bg fixed inset-0 z-50 flex flex-col items-center justify-center select-none"
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.5 }}
    >
      <div className="flex flex-col items-center gap-3">
        <AvatarPlaceholder />

        <p
          className="text-lg font-medium text-white"
          style={{ fontFamily: "var(--cros-font-family)" }}
        >
          {username}
        </p>

        <form onSubmit={handleSubmit} className="flex items-center gap-2 mt-2">
          <motion.div animate={shakeControls} className="relative">
            <input
              type="password"
              value={pin}
              onChange={handlePinChange}
              placeholder="PIN"
              maxLength={MAX_PIN_LENGTH}
              autoFocus
              className="w-52 px-5 py-2.5 text-sm text-white placeholder-white/50 outline-none"
              style={{
                background: "rgba(255, 255, 255, 0.12)",
                border: "1px solid rgba(255, 255, 255, 0.3)",
                borderRadius: "var(--cros-radius-pill)",
                backdropFilter: "var(--cros-blur)",
                fontFamily: "var(--cros-font-family-text)",
                transition: "var(--cros-transition-normal)",
              }}
            />
          </motion.div>

          {pin.length > 0 && (
            <motion.button
              type="submit"
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ duration: 0.15 }}
              className="flex items-center justify-center w-10 h-10 text-white cursor-pointer"
              style={{
                background: "var(--cros-sys-primary)",
                borderRadius: "var(--cros-radius-pill)",
                border: "none",
                transition: "var(--cros-transition-normal)",
              }}
              aria-label="Submit PIN"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M5 12h14" />
                <path d="M13 6l6 6-6 6" />
              </svg>
            </motion.button>
          )}
        </form>
      </div>

      {/* System tray - bottom right */}
      <div className="absolute bottom-4 right-4 flex items-center gap-3 text-white/80 text-xs">
        {wifiEnabled && <WifiIcon />}
        <BatteryIcon level={batteryLevel} />
        <span style={{ fontFamily: "var(--cros-font-family-text)" }}>
          {time}
        </span>
      </div>
    </motion.div>
  );
}
