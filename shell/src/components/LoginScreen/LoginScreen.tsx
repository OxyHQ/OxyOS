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

      {/* Main content — centered */}
      <div className="flex flex-1 flex-col items-center justify-center">
        {/* Clock */}
        <div className="-mt-[6vh] mb-10 flex flex-col items-center">
          <p className="text-[15px] font-medium tracking-wide text-white/50">
            {formattedDate}
          </p>
          <p className="mt-1 text-[min(96px,11vw)] font-bold leading-[0.85] tracking-[-2px] text-white">
            {time || "0:00"}
          </p>
        </div>

        {/* Circular login form */}
        <form onSubmit={handleSubmit} className="relative">
          {/* Circle outline */}
          <div className="flex h-[280px] w-[280px] flex-col items-center justify-between rounded-full border border-white/20 px-10 py-8">
            {/* Avatar pill */}
            <div className="flex h-10 w-20 items-center justify-center rounded-full bg-white/15">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="rgba(255,255,255,0.7)">
                <circle cx="12" cy="8" r="4" />
                <path d="M4 20c0-4 4-7 8-7s8 3 8 7" />
              </svg>
            </div>

            {/* Username pill */}
            <div className="flex h-11 w-full items-center justify-center rounded-full bg-white/12">
              <span className="text-[14px] font-medium text-white/80">{username}</span>
            </div>

            {/* Password input */}
            <motion.div animate={shakeControls} className="w-full">
              <input
                type="password"
                value={password}
                onChange={(e) => {
                  setPassword(e.target.value);
                  setShowError(false);
                }}
                placeholder="Password"
                autoFocus
                className="h-11 w-full rounded-full border-none bg-white/12 px-5 text-center text-[14px] text-white placeholder-white/30 outline-none transition-all duration-200 focus:bg-white/18"
              />
            </motion.div>

            {/* Submit button */}
            <motion.button
              type="submit"
              className="flex h-12 w-full cursor-pointer items-center justify-center rounded-full bg-[rgba(32,33,36,0.85)] text-white transition-colors duration-150 hover:bg-[rgba(32,33,36,0.95)]"
              whileTap={{ scale: 0.97 }}
            >
              <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </motion.button>
          </div>
        </form>

        {/* Status text below circle */}
        <AnimatePresence mode="wait">
          {showError ? (
            <motion.p
              key="error"
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="mt-5 text-[12px] text-white/50"
            >
              Incorrect password. Try again.
            </motion.p>
          ) : (
            <motion.p
              key="hint"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="mt-5 text-[12px] text-white/35"
            >
              Enter password to sign in
            </motion.p>
          )}
        </AnimatePresence>
      </div>
    </motion.div>
  );
}
