import { useState, useCallback } from "react";
import { motion, useAnimation, AnimatePresence } from "framer-motion";
import { useSessionStore } from "../../stores/sessionStore";
import { useSystemStore } from "../../stores/systemStore";

const CORRECT_PASSWORD = "1234";

function WifiIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
      <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
    </svg>
  );
}

function BatteryIcon({ level }: { level: number }) {
  const fillWidth = Math.round((level / 100) * 14);
  return (
    <svg width="20" height="12" viewBox="0 0 22 12" fill="none">
      <rect
        x="0.5"
        y="0.5"
        width="18"
        height="11"
        rx="2"
        stroke="currentColor"
        strokeWidth="1"
      />
      <rect
        x="2"
        y="2"
        width={fillWidth}
        height="8"
        rx="1"
        fill="currentColor"
      />
      <rect x="19" y="3" width="2.5" height="6" rx="1" fill="currentColor" />
    </svg>
  );
}

function PowerIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
      <line x1="12" y1="2" x2="12" y2="12" />
    </svg>
  );
}

function UserSilhouette() {
  return (
    <svg
      width="36"
      height="36"
      viewBox="0 0 24 24"
      fill="rgba(255,255,255,0.8)"
    >
      <circle cx="12" cy="8" r="4" />
      <path d="M4 20c0-4 4-7 8-7s8 3 8 7" />
    </svg>
  );
}

function HelpIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="12" cy="12" r="10" />
      <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
      <circle cx="12" cy="17" r="0.5" fill="currentColor" />
    </svg>
  );
}

export default function LoginScreen() {
  const [password, setPassword] = useState("");
  const [showError, setShowError] = useState(false);
  const username = useSessionStore((s) => s.username);
  const time = useSystemStore((s) => s.time);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const shakeControls = useAnimation();

  const formattedDate = new Date().toLocaleDateString("en-US", {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  const handleSubmit = useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (password === CORRECT_PASSWORD) {
        setShowError(false);
        useSessionStore.getState().login();
      } else {
        shakeControls.start({
          x: [0, -12, 12, -8, 8, -4, 4, 0],
          transition: { duration: 0.4 },
        });
        setPassword("");
        setShowError(true);
      }
    },
    [password, shakeControls],
  );

  return (
    <motion.div
      className="wallpaper-bg fixed inset-0 z-50 flex flex-col select-none"
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.5 }}
    >
      {/* System tray */}
      <div className="absolute top-4 right-6 flex items-center gap-3 text-white/70">
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10"
        >
          <WifiIcon />
        </button>
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10"
        >
          <BatteryIcon level={batteryLevel} />
        </button>
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10"
        >
          <PowerIcon />
        </button>
      </div>

      {/* Clock */}
      <div className="flex flex-col items-center pt-[14vh]">
        <p className="text-[17px] font-medium tracking-wide text-white/60">
          {formattedDate}
        </p>
        <p className="mt-1 font-bold leading-none tracking-[-3px] text-white text-[clamp(96px,13vw,144px)]">
          {time || "0:00"}
        </p>
      </div>

      {/* Login */}
      <div className="-mt-8 flex flex-1 flex-col items-center justify-center gap-5">
        <div className="flex h-20 w-20 items-center justify-center rounded-full bg-white/15 ring-[2.5px] ring-white/25 backdrop-blur-xl">
          <UserSilhouette />
        </div>

        <p className="mt-4 text-[17px] font-medium text-white">{username}</p>

        <form
          onSubmit={handleSubmit}
          className="mt-5 flex items-center gap-3"
        >
          <motion.div animate={shakeControls}>
            <input
              type="password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setShowError(false);
              }}
              placeholder="Enter password"
              autoFocus
              className="h-[40px] w-[240px] rounded-full border border-white/20 bg-white/10 px-5 text-[14px] text-white placeholder-white/35 outline-none backdrop-blur-xl transition-all duration-200 focus:border-white/40 focus:bg-white/15"
            />
          </motion.div>
          <button
            type="button"
            className="flex h-[34px] w-[34px] cursor-pointer items-center justify-center rounded-full text-white/40 transition-all duration-200 hover:bg-white/8 hover:text-white/70"
            aria-label="Help"
          >
            <HelpIcon />
          </button>
        </form>

        <AnimatePresence mode="wait">
          {showError ? (
            <motion.p
              key="error"
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="mt-4 text-[13px] text-white/40"
            >
              Incorrect password. Try again.
            </motion.p>
          ) : (
            <motion.p
              key="hint"
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="mt-4 text-[13px] text-white/40"
            >
              Your password is required to log in
            </motion.p>
          )}
        </AnimatePresence>
      </div>
    </motion.div>
  );
}
