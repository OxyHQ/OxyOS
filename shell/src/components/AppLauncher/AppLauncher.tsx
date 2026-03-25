import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";

interface AppEntry {
  name: string;
  color: string;
  icon: React.ReactNode;
}

function SearchIcon() {
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
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.35-4.35" />
    </svg>
  );
}

const apps: AppEntry[] = [
  {
    name: "Chrome",
    color: "#4285f4",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <circle cx="12" cy="12" r="4" />
        <path
          d="M12 2a10 10 0 0 1 8.66 5h-5.66a5 5 0 0 0-4.33-2.5L12 2z"
          opacity="0.9"
        />
        <path
          d="M3.34 7A10 10 0 0 0 2 12a10 10 0 0 0 3.34 7.5l2.83-4.9A5 5 0 0 1 7 12L3.34 7z"
          opacity="0.8"
        />
        <path
          d="M12 22a10 10 0 0 0 8.66-5h-5.49a5 5 0 0 1-3.17 1.5L12 22z"
          opacity="0.7"
        />
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
        <path
          d="M3 5l9 7 9-7"
          fill="none"
          stroke="white"
          strokeWidth="1.5"
          strokeLinejoin="round"
        />
      </svg>
    ),
  },
  {
    name: "YouTube",
    color: "#ea4335",
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
        <path d="M3 9h18" stroke="rgba(0,0,0,0.2)" strokeWidth="1" />
        <rect x="7" y="12" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.25)" />
        <rect x="14" y="12" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.25)" />
        <rect x="7" y="16.5" width="3" height="3" rx="0.5" fill="rgba(0,0,0,0.25)" />
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
        <circle cx="12" cy="10" r="3" fill="#ea4335" />
        <circle cx="8.5" cy="14" r="3" fill="#4285f4" />
        <circle cx="15.5" cy="14" r="3" fill="#34a853" />
        <circle cx="12" cy="12" r="2" fill="white" />
      </svg>
    ),
  },
  {
    name: "Docs",
    color: "#4285f4",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M6 2h9l5 5v15a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z" />
        <path d="M15 2v5h5" opacity="0.5" />
        <line x1="8" y1="12" x2="16" y2="12" stroke="rgba(0,0,0,0.25)" strokeWidth="1.5" />
        <line x1="8" y1="15" x2="14" y2="15" stroke="rgba(0,0,0,0.25)" strokeWidth="1.5" />
        <line x1="8" y1="18" x2="12" y2="18" stroke="rgba(0,0,0,0.25)" strokeWidth="1.5" />
      </svg>
    ),
  },
  {
    name: "Music",
    color: "#f28b00",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M9 17V7l10-2v12" />
        <circle cx="7" cy="17" r="3" />
        <circle cx="17" cy="15" r="3" />
      </svg>
    ),
  },
  {
    name: "Camera",
    color: "#5f6368",
    icon: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="white">
        <path d="M4 7h3l2-3h6l2 3h3a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z" />
        <circle cx="12" cy="13" r="4" fill="rgba(0,0,0,0.25)" />
        <circle cx="12" cy="13" r="2.5" fill="white" />
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
          initial={{ y: 100, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 100, opacity: 0 }}
          transition={panelSpring}
          className="fixed left-1/2 -translate-x-1/2 z-40 flex flex-col overflow-hidden select-none"
          style={{
            bottom: 56,
            width: 640,
            height: 480,
            borderRadius: "var(--cros-radius-l)",
            background: "rgba(32, 33, 36, 0.88)",
            backdropFilter: "blur(64px) saturate(180%)",
          }}
        >
          {/* Search bar */}
          <div className="px-6 pt-6 pb-2">
            <div
              className="flex items-center gap-3 px-4 py-2.5"
              style={{
                background: "rgba(255, 255, 255, 0.08)",
                borderRadius: "var(--cros-radius-pill)",
              }}
            >
              <span className="text-white/60 shrink-0">
                <SearchIcon />
              </span>
              <input
                type="text"
                placeholder="Search your device"
                className="w-full bg-transparent text-sm text-white placeholder-white/40 outline-none"
                style={{ fontFamily: "var(--cros-font-family-text)" }}
              />
            </div>
          </div>

          {/* App grid */}
          <div
            className="grid gap-4 px-6 py-6 overflow-y-auto"
            style={{ gridTemplateColumns: "repeat(5, 1fr)" }}
          >
            {apps.map((app) => (
              <button
                key={app.name}
                className="group flex flex-col items-center gap-2 bg-transparent border-none cursor-pointer p-0"
                type="button"
              >
                <div
                  className="flex items-center justify-center w-16 h-16 rounded-full transition-transform transition-[filter] duration-150 group-hover:scale-105 group-hover:brightness-110"
                  style={{ background: app.color }}
                >
                  {app.icon}
                </div>
                <span
                  className="text-white text-xs leading-tight truncate max-w-[72px] text-center"
                  style={{
                    fontSize: 12,
                    fontFamily: "var(--cros-font-family-text)",
                  }}
                >
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
