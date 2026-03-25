import { useState, useCallback } from "react";
import { motion, useAnimation, AnimatePresence } from "framer-motion";
import { useSessionStore } from "../../stores/sessionStore";
import { useSystemStore } from "../../stores/systemStore";

const CORRECT_PASSWORD = "1234";

export default function LoginScreen() {
  const [password, setPassword] = useState("");
  const [showError, setShowError] = useState(false);
  const username = useSessionStore((s) => s.username);
  const time = useSystemStore((s) => s.time);
  const batteryLevel = useSystemStore((s) => s.batteryLevel);
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
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
      {/* Top-right system tray */}
      <div className="absolute top-3 right-5 flex items-center gap-1 text-white/70">
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill={wifiEnabled ? "currentColor" : "#9aa0a6"}>
            <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
          </svg>
        </button>
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
        >
          <svg width="20" height="11" viewBox="0 0 22 12" fill="none">
            <rect x="0.5" y="0.5" width="18" height="11" rx="2" stroke="currentColor" strokeWidth="1" />
            <rect x="2" y="2" width={Math.round((batteryLevel / 100) * 14)} height="8" rx="1" fill="currentColor" />
            <rect x="19" y="3" width="2.5" height="6" rx="1" fill="currentColor" />
          </svg>
        </button>
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
            <line x1="12" y1="2" x2="12" y2="12" />
          </svg>
        </button>
      </div>

      {/* Centered content — clock on top third, login in lower center */}
      <div className="flex flex-1 flex-col items-center">
        {/* Clock area — upper portion */}
        <div className="flex flex-col items-center pt-[10vh]">
          <p className="text-[16px] font-medium tracking-wide text-white/55">
            {formattedDate}
          </p>
          <p className="mt-1 text-[min(120px,12vw)] font-bold leading-[0.85] tracking-[-2px] text-white">
            {time || "0:00"}
          </p>
        </div>

        {/* Spacer between clock and login — generous breathing room */}
        <div className="flex-1 min-h-[8vh] max-h-[14vh]" />

        {/* Avatar */}
        <div className="flex h-[72px] w-[72px] items-center justify-center rounded-full bg-white/15 ring-[2px] ring-white/20 backdrop-blur-xl">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="rgba(255,255,255,0.8)">
            <circle cx="12" cy="8" r="4" />
            <path d="M4 20c0-4 4-7 8-7s8 3 8 7" />
          </svg>
        </div>

        {/* Username */}
        <p className="mt-3 text-[16px] font-medium text-white">{username}</p>

        {/* Password form */}
        <form onSubmit={handleSubmit} className="mt-4 flex items-center gap-2.5">
          <motion.div animate={shakeControls}>
            <input
              type="password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setShowError(false);
              }}
              placeholder="Enter Password"
              autoFocus
              className="h-[38px] w-[220px] rounded-full border border-white/20 bg-white/10 px-5 text-[13px] text-white placeholder-white/35 outline-none backdrop-blur-xl transition-all duration-200 focus:border-white/40 focus:bg-white/[0.14]"
            />
          </motion.div>
          <button
            type="button"
            className="flex h-[30px] w-[30px] cursor-pointer items-center justify-center rounded-full text-white/35 transition-all duration-150 hover:bg-white/10 hover:text-white/60"
            aria-label="Help"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="10" />
              <path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" />
              <circle cx="12" cy="17" r="0.5" fill="currentColor" />
            </svg>
          </button>
        </form>

        {/* Status text */}
        <AnimatePresence mode="wait">
          {showError ? (
            <motion.p
              key="error"
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="mt-3 text-[12px] text-white/45"
            >
              Incorrect password. Try again.
            </motion.p>
          ) : (
            <motion.p
              key="hint"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="mt-3 text-[12px] text-white/40"
            >
              Your password is required to log in
            </motion.p>
          )}
        </AnimatePresence>

        {/* Bottom spacer to push login area up from absolute bottom */}
        <div className="flex-1 min-h-[10vh]" />
      </div>
    </motion.div>
  );
}
