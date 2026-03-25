import { useState, useCallback } from "react";
import { motion, useAnimation, AnimatePresence } from "framer-motion";
import { useSessionStore } from "../../stores/sessionStore";
import { useSystemStore } from "../../stores/systemStore";
import avatarDefault from "../../assets/avatar-default.png";

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
      {/* ── Top-right system tray ── */}
      <div className="absolute top-2.5 right-4 flex items-center gap-0.5 text-white/75">
        <span className="px-1.5 text-[12px] font-medium">U.S.</span>
        <button type="button" className="flex h-7 w-7 items-center justify-center rounded-full hover:bg-white/10">
          <svg width="15" height="15" viewBox="0 0 24 24" fill={wifiEnabled ? "currentColor" : "#9aa0a6"}>
            <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
          </svg>
        </button>
        <svg width="22" height="11" viewBox="0 0 24 13" fill="none" className="ml-0.5">
          <rect x="0.5" y="0.5" width="20" height="12" rx="2.5" stroke="currentColor" strokeWidth="1" />
          <rect x="2" y="2" width={Math.round((batteryLevel / 100) * 16)} height="9" rx="1.5" fill="currentColor" />
          <path d="M22 4.5c.83 0 1.5.9 1.5 2s-.67 2-1.5 2" stroke="currentColor" strokeWidth="1" />
        </svg>
        <button type="button" className="ml-0.5 flex h-7 w-7 items-center justify-center rounded-full hover:bg-white/10">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
            <line x1="12" y1="2" x2="12" y2="12" />
          </svg>
        </button>
      </div>

      {/* ── Clock — upper area ── */}
      <div className="mt-[18vh] flex flex-col items-center">
        <p className="text-[17px] font-normal tracking-[0.02em] text-white/65">
          {formattedDate}
        </p>
        <p className="mt-1.5 text-[min(128px,14vw)] font-bold leading-[0.88] tracking-[-1px] text-white/80">
          {time || "0:00"}
        </p>
      </div>

      {/* ── Spacer ── */}
      <div className="flex-1" />

      {/* ── Login area — lower-center ── */}
      <div className="flex flex-col items-center pb-[12vh]">
        {/* Avatar */}
        <img
          src={avatarDefault}
          alt="User avatar"
          className="h-12 w-12 rounded-full object-cover ring-[1.5px] ring-white/30"
        />

        {/* Username */}
        <p className="mt-2.5 text-[15px] font-medium text-white">{username}</p>

        {/* Password row */}
        <form onSubmit={handleSubmit} className="mt-4 flex items-center gap-2">
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
              className="h-[34px] w-[200px] rounded-full bg-white/20 px-4 text-[13px] text-white placeholder-white/45 outline-none backdrop-blur-sm transition-colors duration-200 focus:bg-white/25"
            />
          </motion.div>
          <button
            type="button"
            className="flex h-[28px] w-[28px] shrink-0 cursor-pointer items-center justify-center rounded-full border border-white/30 text-white/50 transition-colors duration-150 hover:bg-white/10 hover:text-white/70"
            aria-label="Password hint"
          >
            <span className="text-[13px] font-medium leading-none">?</span>
          </button>
        </form>

        {/* Hint / error text */}
        <AnimatePresence mode="wait">
          {showError ? (
            <motion.p
              key="error"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="mt-3.5 text-center text-[12px] leading-snug text-white/50"
            >
              Incorrect password. Try again.
            </motion.p>
          ) : (
            <motion.p
              key="hint"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="mt-3.5 text-center text-[12px] leading-snug text-white/45"
            >
              Your password is required to
              <br />
              log in
            </motion.p>
          )}
        </AnimatePresence>
      </div>
    </motion.div>
  );
}
