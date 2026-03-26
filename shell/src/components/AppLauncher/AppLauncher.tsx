import { useState, useMemo, useCallback, useRef, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";
import { invoke } from "../../lib/tauri";
import { appExecMap } from "../../lib/appRegistry";
import { useInstalledApps } from "../../hooks/useInstalledApps";

import browserIcon from "../../assets/icons/browser.svg";
import mailIcon from "../../assets/icons/mail.svg";
import filesIcon from "../../assets/icons/files.svg";
import mapsIcon from "../../assets/icons/maps.svg";
import calendarIcon from "../../assets/icons/calendar.svg";
import photosIcon from "../../assets/icons/photos.svg";
import cameraIcon from "../../assets/icons/camera.svg";
import settingsIcon from "../../assets/icons/settings.svg";
import storeIcon from "../../assets/icons/store.svg";
import terminalIcon from "../../assets/icons/terminal.svg";
import messagesIcon from "../../assets/icons/messages.svg";
import calculatorIcon from "../../assets/icons/calculator.svg";
import clockIcon from "../../assets/icons/clock.svg";
import radioIcon from "../../assets/icons/radio.svg";
import notesIcon from "../../assets/icons/notes.svg";
import docsIcon from "../../assets/icons/docs.svg";

interface AppEntry {
  name: string;
  icon: string;
}

const apps: AppEntry[] = [
  { name: "Browser", icon: browserIcon },
  { name: "Email", icon: mailIcon },
  { name: "Files", icon: filesIcon },
  { name: "Maps", icon: mapsIcon },
  { name: "Calendar", icon: calendarIcon },
  { name: "Photos", icon: photosIcon },
  { name: "Camera", icon: cameraIcon },
  { name: "Settings", icon: settingsIcon },
  { name: "Store", icon: storeIcon },
  { name: "Terminal", icon: terminalIcon },
  { name: "Messages", icon: messagesIcon },
  { name: "Calculator", icon: calculatorIcon },
  { name: "Clock", icon: clockIcon },
  { name: "Radio", icon: radioIcon },
  { name: "Notes", icon: notesIcon },
  { name: "Docs", icon: docsIcon },
];

export default function AppLauncher() {
  const isOpen = useLauncherStore((s) => s.isOpen);
  const close = useLauncherStore((s) => s.close);
  const { apps: installedApps } = useInstalledApps();
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset search when closing
  useEffect(() => {
    if (!isOpen) setQuery("");
  }, [isOpen]);

  // Auto-focus search when opening
  useEffect(() => {
    if (isOpen) {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [isOpen]);

  const displayApps = useMemo(() =>
    installedApps.length > 0
      ? installedApps.map((a) => ({ name: a.name, icon: a.icon, exec: a.exec }))
      : apps.map((a) => ({ name: a.name, icon: a.icon, exec: appExecMap[a.name] || "" })),
    [installedApps],
  );

  const filtered = useMemo(() => {
    if (!query.trim()) return displayApps;
    const q = query.toLowerCase();
    return displayApps.filter((a) => a.name.toLowerCase().includes(q));
  }, [displayApps, query]);

  const getIconSrc = useCallback((icon: string) => {
    if (icon.startsWith("/") || icon.startsWith("data:") || icon.startsWith("http")) return icon;
    if (!icon.includes(".")) return `/usr/share/icons/hicolor/256x256/apps/${icon}.png`;
    return icon;
  }, []);

  const handleLaunch = (exec: string) => {
    if (exec) {
      invoke("launch_app", { exec });
      close();
    }
  };

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          {/* Full-screen blurred backdrop */}
          <motion.div
            key="launcher-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="fixed inset-0 z-35 bg-black/50 backdrop-blur-[32px]"
            onClick={close}
          />

          {/* App grid */}
          <motion.div
            key="launcher-grid"
            initial={{ opacity: 0, scale: 0.92 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
            className="fixed inset-x-0 top-0 bottom-[56px] z-40 flex flex-col items-center select-none"
          >
            {/* Search bar — Liquid Glass */}
            <div className="mt-[6vh] w-[420px]">
              <div className="flex h-[42px] items-center gap-2.5 rounded-full border border-white/15 bg-white/10 px-4 shadow-[0_2px_12px_rgba(0,0,0,0.15),inset_0_0.5px_0_rgba(255,255,255,0.1)] backdrop-blur-[40px]">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-40">
                  <circle cx="11" cy="11" r="7" />
                  <path d="M21 21l-4.35-4.35" />
                </svg>
                <input
                  ref={inputRef}
                  type="text"
                  placeholder="Search apps..."
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  className="min-w-0 flex-1 bg-transparent text-[14px] text-white placeholder-white/35 outline-none"
                />
                {query && (
                  <button
                    onClick={() => setQuery("")}
                    className="flex h-5 w-5 cursor-pointer items-center justify-center rounded-full bg-white/15 text-white/50 transition-colors hover:bg-white/25 hover:text-white/80"
                  >
                    <svg width="8" height="8" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                      <path d="M1 1l8 8M9 1l-8 8" />
                    </svg>
                  </button>
                )}
              </div>
            </div>

            {/* Apps grid */}
            <div className="mt-[5vh] flex-1 overflow-y-auto overflow-x-hidden px-8 pb-4 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
              {filtered.length > 0 ? (
                <div className="grid grid-cols-6 gap-x-10 gap-y-6">
                  {filtered.map((app, i) => (
                    <motion.button
                      key={app.name}
                      type="button"
                      initial={{ opacity: 0, y: 12 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ duration: 0.15, delay: Math.min(i * 0.02, 0.3), ease: [0.2, 0, 0, 1] }}
                      whileTap={{ scale: 0.85 }}
                      onClick={() => handleLaunch(app.exec)}
                      className="flex cursor-pointer flex-col items-center gap-2 border-none bg-transparent p-0"
                    >
                      <div className="flex h-[60px] w-[60px] items-center justify-center overflow-hidden rounded-full border border-white/10 bg-white/8 shadow-[0_2px_8px_rgba(0,0,0,0.15)] backdrop-blur-sm transition-transform duration-100">
                        <img
                          src={getIconSrc(app.icon)}
                          alt={app.name}
                          className="h-full w-full object-cover"
                          draggable={false}
                          onError={(e) => {
                            const img = e.currentTarget;
                            img.style.display = "none";
                            const parent = img.parentElement;
                            if (parent && !parent.querySelector("span")) {
                              const span = document.createElement("span");
                              span.className = "text-[22px] font-bold text-white/80";
                              span.textContent = app.name.charAt(0);
                              parent.appendChild(span);
                            }
                          }}
                        />
                      </div>
                      <span className="max-w-[76px] truncate text-center text-[11px] font-medium leading-tight text-white/80 drop-shadow-[0_1px_2px_rgba(0,0,0,0.5)]">
                        {app.name}
                      </span>
                    </motion.button>
                  ))}
                </div>
              ) : (
                <div className="flex flex-col items-center pt-16">
                  <div className="mb-3 flex h-14 w-14 items-center justify-center rounded-full bg-white/8">
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" opacity="0.3">
                      <circle cx="11" cy="11" r="7" />
                      <path d="M21 21l-4.35-4.35" />
                    </svg>
                  </div>
                  <p className="text-[14px] font-medium text-white/40">No apps found</p>
                  <p className="mt-1 text-[12px] text-white/25">Try a different search</p>
                </div>
              )}
            </div>

            {/* Page dots */}
            <div className="mt-auto mb-6 flex items-center gap-1.5">
              <div className="h-1.5 w-1.5 rounded-full bg-white" />
              <div className="h-1.5 w-1.5 rounded-full bg-white/30" />
              <div className="h-1.5 w-1.5 rounded-full bg-white/30" />
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
