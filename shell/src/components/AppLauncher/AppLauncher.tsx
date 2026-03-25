import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";

interface AppEntry {
  name: string;
  color: string;
  icon: React.ReactNode;
}

const apps: AppEntry[] = [
  {
    name: "Chrome",
    color: "#4285f4",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2a10 10 0 0 1 8.66 5h-5.66a5 5 0 0 0-4.33-2.5L12 2z" opacity="0.9" />
        <path d="M3.34 7A10 10 0 0 0 2 12a10 10 0 0 0 3.34 7.5l2.83-4.9A5 5 0 0 1 7 12L3.34 7z" opacity="0.8" />
        <path d="M12 22a10 10 0 0 0 8.66-5h-5.49a5 5 0 0 1-3.17 1.5L12 22z" opacity="0.7" />
      </svg>
    ),
  },
  {
    name: "Play Store",
    color: "#34a853",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M4 2l12 10L4 22V2z" />
        <path d="M4 2l12 10-4 3.33L4 8V2z" opacity="0.8" />
      </svg>
    ),
  },
  {
    name: "Files",
    color: "#4285f4",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M4 4h6l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" />
      </svg>
    ),
  },
  {
    name: "Gmail",
    color: "#ea4335",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <rect x="3" y="5" width="18" height="14" rx="2" />
        <path d="M3 5l9 7 9-7" fill="none" stroke="rgba(0,0,0,0.2)" strokeWidth="1.5" strokeLinejoin="round" />
      </svg>
    ),
  },
  {
    name: "YouTube",
    color: "#ff0000",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <rect x="2" y="4" width="20" height="16" rx="4" opacity="0.3" />
        <polygon points="10,8 10,16 17,12" />
      </svg>
    ),
  },
  {
    name: "Maps",
    color: "#34a853",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7z" />
        <circle cx="12" cy="9" r="2.5" fill="rgba(0,0,0,0.3)" />
      </svg>
    ),
  },
  {
    name: "Calendar",
    color: "#4285f4",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <rect x="3" y="4" width="18" height="18" rx="2" />
        <rect x="7" y="11" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.2)" />
        <rect x="14" y="11" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.2)" />
        <rect x="7" y="16" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.2)" />
        <line x1="8" y1="2" x2="8" y2="5" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
        <line x1="16" y1="2" x2="16" y2="5" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    name: "Photos",
    color: "#fbbc04",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24">
        <circle cx="12" cy="10" r="3.5" fill="#ea4335" />
        <circle cx="8" cy="14.5" r="3.5" fill="#4285f4" />
        <circle cx="16" cy="14.5" r="3.5" fill="#34a853" />
        <circle cx="12" cy="12.5" r="2" fill="white" />
      </svg>
    ),
  },
  {
    name: "Camera",
    color: "#5f6368",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M4 7h3l2-3h6l2 3h3a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z" />
        <circle cx="12" cy="13" r="4" fill="rgba(0,0,0,0.2)" />
        <circle cx="12" cy="13" r="2.5" fill="white" />
      </svg>
    ),
  },
  {
    name: "Settings",
    color: "#5f6368",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54A.48.48 0 0013.93 2h-3.86a.48.48 0 00-.48.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.71 8.81a.48.48 0 00.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.26.41.48.41h3.86c.22 0 .43-.17.48-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.6 3.6 0 0112 15.6z" />
      </svg>
    ),
  },
];

const panelSpring = {
  type: "spring" as const,
  stiffness: 300,
  damping: 30,
};

export default function AppLauncher() {
  const isOpen = useLauncherStore((s) => s.isOpen);

  return (
    <AnimatePresence>
      {isOpen && (
        <motion.div
          key="app-launcher"
          initial={{ y: 60, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 60, opacity: 0 }}
          transition={panelSpring}
          className="fixed bottom-[56px] left-3 z-40 flex w-[480px] flex-col overflow-hidden rounded-[24px] bg-white/95 shadow-[0_8px_40px_rgba(0,0,0,0.28)] backdrop-blur-[40px] backdrop-saturate-[180%] select-none"
        >
          {/* Search bar */}
          <div className="px-4 pt-5 pb-2">
            <div className="flex h-[48px] items-center gap-3 rounded-full bg-[#f1f3f4] px-4">
              {/* Google G icon placeholder */}
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" className="shrink-0">
                <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285f4" />
                <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34a853" />
                <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#fbbc05" />
                <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#ea4335" />
              </svg>
              <input
                type="text"
                placeholder="Search your settings, files, apps, and more..."
                className="min-w-0 flex-1 bg-transparent text-[14px] text-[#202124] placeholder-[#5f6368] outline-none"
              />
            </div>
          </div>

          {/* App grid */}
          <div className="grid grid-cols-5 gap-y-2 px-4 pt-3 pb-5">
            {apps.map((app) => (
              <button
                key={app.name}
                type="button"
                className="group flex cursor-pointer flex-col items-center gap-2 border-none bg-transparent px-2 py-3 rounded-[16px] transition-colors duration-150 hover:bg-[#e8eaed]"
              >
                <div
                  className="flex h-[52px] w-[52px] items-center justify-center rounded-full transition-transform duration-150 group-hover:scale-105"
                  style={{ backgroundColor: app.color }}
                >
                  {app.icon}
                </div>
                <span className="max-w-[72px] truncate text-center text-[11px] leading-tight text-[#3c4043]">
                  {app.name}
                </span>
              </button>
            ))}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
