import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";

interface AppEntry {
  name: string;
  color: string;
  icon: React.ReactNode;
}

const pinnedApps: AppEntry[] = [
  {
    name: "Chrome",
    color: "#4285f4",
    icon: (
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none">
        <circle cx="12" cy="12" r="10" stroke="white" strokeWidth="1.5" />
        <ellipse cx="12" cy="12" rx="4" ry="10" stroke="white" strokeWidth="1.5" />
        <line x1="2" y1="12" x2="22" y2="12" stroke="white" strokeWidth="1.5" />
      </svg>
    ),
  },
  {
    name: "Discord",
    color: "#5865f2",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M19.27 5.33C17.94 4.71 16.5 4.26 15 4a.09.09 0 0 0-.07.03c-.18.33-.39.76-.53 1.09a16.09 16.09 0 0 0-4.8 0c-.14-.34-.36-.76-.54-1.09-.01-.02-.04-.03-.07-.03-1.5.26-2.93.71-4.27 1.33-.01 0-.02.01-.03.02-2.72 4.07-3.47 8.03-3.1 11.95 0 .02.01.04.03.05 1.8 1.32 3.53 2.12 5.24 2.65.03.01.06 0 .07-.02.4-.55.76-1.13 1.07-1.74.02-.04 0-.08-.04-.09-.57-.22-1.11-.48-1.64-.78-.04-.02-.04-.08-.01-.11.11-.08.22-.17.33-.25.02-.02.05-.02.07-.01 3.44 1.57 7.15 1.57 10.55 0 .02-.01.05-.01.07.01.11.09.22.17.33.26.04.03.04.09-.01.11-.52.31-1.07.56-1.64.78-.04.01-.05.06-.04.09.32.61.68 1.19 1.07 1.74.03.01.05.02.07.02 1.72-.53 3.45-1.33 5.24-2.65.02-.01.03-.03.03-.05.44-4.53-.73-8.46-3.1-11.95-.01-.01-.02-.02-.04-.02zM8.52 14.91c-1.03 0-1.89-.95-1.89-2.12s.84-2.12 1.89-2.12c1.06 0 1.9.96 1.89 2.12 0 1.17-.84 2.12-1.89 2.12zm6.97 0c-1.03 0-1.89-.95-1.89-2.12s.84-2.12 1.89-2.12c1.06 0 1.9.96 1.89 2.12 0 1.17-.83 2.12-1.89 2.12z" />
      </svg>
    ),
  },
  {
    name: "Spotify",
    color: "#1db954",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm4.64 14.36c-.2.2-.51.2-.71 0-1.95-1.19-4.41-1.46-7.31-.8-.28.07-.55-.1-.62-.38-.07-.28.1-.55.38-.62 3.17-.72 5.89-.41 8.07.93.2.13.26.4.13.6l.06.27zm1.23-2.71c-.25.25-.62.25-.87 0-2.23-1.37-5.63-1.77-8.27-.97-.33.1-.68-.09-.78-.42-.1-.33.09-.68.42-.78 3.01-.91 6.75-.47 9.29 1.1.25.16.32.5.16.75l.05.32zm.11-2.82c-2.68-1.59-7.1-1.74-9.66-.96-.41.13-.84-.1-.97-.51-.13-.41.1-.84.51-.97 2.93-.89 7.81-.72 10.9 1.11.37.22.49.69.27 1.06-.22.37-.69.49-1.05.27z" />
      </svg>
    ),
  },
  {
    name: "YouTube",
    color: "#ff0000",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M10 15l5.19-3L10 9v6m11.56-7.83c.13.47.22 1.1.28 1.9.07.8.1 1.49.1 2.09L22 12c0 2.19-.16 3.8-.44 4.83-.25.9-.83 1.48-1.73 1.73-.47.13-1.33.22-2.65.28-1.3.07-2.49.1-3.59.1L12 19c-4.19 0-6.8-.16-7.83-.44-.9-.25-1.48-.83-1.73-1.73-.13-.47-.22-1.1-.28-1.9-.07-.8-.1-1.49-.1-2.09L2 12c0-2.19.16-3.8.44-4.83.25-.9.83-1.48 1.73-1.73.47-.13 1.33-.22 2.65-.28 1.3-.07 2.49-.1 3.59-.1L12 5c4.19 0 6.8.16 7.83.44.9.25 1.48.83 1.73 1.73z" />
      </svg>
    ),
  },
  {
    name: "Gmail",
    color: "#ea4335",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4l-8 5-8-5V6l8 5 8-5v2z" />
      </svg>
    ),
  },
  {
    name: "Files",
    color: "#4285f4",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M4 4h6l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" />
      </svg>
    ),
  },
  {
    name: "Maps",
    color: "#34a853",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7zm0 9.5a2.5 2.5 0 1 1 0-5 2.5 2.5 0 0 1 0 5z" />
      </svg>
    ),
  },
  {
    name: "Twitter",
    color: "#1da1f2",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M22.46 6c-.77.35-1.6.58-2.46.69.88-.53 1.56-1.37 1.88-2.38-.83.5-1.75.85-2.72 1.05C18.37 4.5 17.26 4 16 4c-2.35 0-4.27 1.92-4.27 4.29 0 .34.04.67.11.98C8.28 9.09 5.11 7.38 3 4.79c-.37.63-.58 1.37-.58 2.15 0 1.49.75 2.81 1.91 3.56-.71 0-1.37-.2-1.95-.5v.03c0 2.08 1.48 3.82 3.44 4.21a4.22 4.22 0 0 1-1.93.07 4.28 4.28 0 0 0 4 2.98 8.521 8.521 0 0 1-5.33 1.84c-.34 0-.68-.02-1.02-.06C3.44 20.29 5.7 21 8.12 21 16 21 20.33 14.46 20.33 8.79c0-.19 0-.37-.01-.56.84-.6 1.56-1.36 2.14-2.23z" />
      </svg>
    ),
  },
  {
    name: "Facebook",
    color: "#1877f2",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M13.397 20.997v-8.196h2.765l.411-3.209h-3.176V7.548c0-.926.258-1.56 1.587-1.56h1.684V3.127A22.336 22.336 0 0 0 14.201 3c-2.444 0-4.122 1.492-4.122 4.231v2.355H7.332v3.209h2.753v8.202h3.312z" />
      </svg>
    ),
  },
  {
    name: "Reddit",
    color: "#ff4500",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <circle cx="12" cy="12" r="10" />
        <circle cx="8.5" cy="12.5" r="1.5" fill="currentColor" />
        <circle cx="15.5" cy="12.5" r="1.5" fill="currentColor" />
        <path d="M8.5 16c1 1.5 5.5 1.5 7 0" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" fill="none" />
        <circle cx="18" cy="5" r="1.5" />
        <line x1="15" y1="3" x2="18" y2="5" stroke="white" strokeWidth="1.5" />
      </svg>
    ),
  },
  {
    name: "Drive",
    color: "#4285f4",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M7.71 3.5L1.15 15l2.86 4.97L10.57 8.47 7.71 3.5zm1.43 0l6.86 11.94H2.58l2.86-4.97L7.71 3.5h1.43zm7.72 11.94L12 3.5h5.72l4.86 8.47-2.86 4.97-2.86-1.5z" />
      </svg>
    ),
  },
  {
    name: "Settings",
    color: "#5f6368",
    icon: (
      <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
        <path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54A.48.48 0 0013.93 2h-3.86a.48.48 0 00-.48.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.71 8.81a.48.48 0 00.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.26.41.48.41h3.86c.22 0 .43-.17.48-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.6 3.6 0 0112 15.6z" />
      </svg>
    ),
  },
];

const panelSpring = {
  type: "spring" as const,
  stiffness: 260,
  damping: 28,
};

export default function AppLauncher() {
  const isOpen = useLauncherStore((s) => s.isOpen);

  return (
    <AnimatePresence>
      {isOpen && (
        <motion.div
          key="app-launcher"
          initial={{ y: 40, opacity: 0, scale: 0.97 }}
          animate={{ y: 0, opacity: 1, scale: 1 }}
          exit={{ y: 40, opacity: 0, scale: 0.97 }}
          transition={panelSpring}
          className="fixed bottom-[56px] left-1/2 z-40 w-[560px] -translate-x-1/2 overflow-hidden rounded-[16px] border border-white/[0.08] bg-[rgba(24,24,28,0.94)] shadow-[0_12px_48px_rgba(0,0,0,0.5),0_0_0_1px_rgba(255,255,255,0.04)] backdrop-blur-[48px] backdrop-saturate-[180%] select-none"
        >
          {/* Search bar */}
          <div className="px-6 pt-6 pb-4">
            <div className="flex h-10 items-center gap-3 rounded-[10px] bg-white/[0.06] px-4">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-40">
                <circle cx="11" cy="11" r="7" />
                <path d="M21 21l-4.35-4.35" />
              </svg>
              <input
                type="text"
                placeholder="Type to search"
                className="min-w-0 flex-1 bg-transparent text-[13px] text-white placeholder-white/30 outline-none"
              />
            </div>
          </div>

          {/* Section header */}
          <div className="flex items-center justify-between px-6 pb-3">
            <span className="text-[13px] font-semibold text-white/90">Pinned</span>
            <button className="flex items-center gap-1 rounded-[8px] bg-white/[0.06] px-2.5 py-1 text-[11px] font-medium text-white/60 transition-colors duration-150 hover:bg-white/10">
              All apps
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M9 18l6-6-6-6" />
              </svg>
            </button>
          </div>

          {/* App grid — 6 columns like Windows 11 */}
          <div className="grid grid-cols-6 gap-y-1 px-4 pb-6">
            {pinnedApps.map((app) => (
              <button
                key={app.name}
                type="button"
                className="flex cursor-pointer flex-col items-center gap-1.5 rounded-[10px] border-none bg-transparent px-1 py-3 transition-colors duration-150 hover:bg-white/[0.06]"
              >
                <div
                  className="flex h-11 w-11 items-center justify-center rounded-[12px]"
                  style={{ backgroundColor: app.color }}
                >
                  {app.icon}
                </div>
                <span className="max-w-[60px] truncate text-center text-[11px] leading-tight text-white/70">
                  {app.name}
                </span>
              </button>
            ))}
          </div>

          {/* Bottom section — user profile */}
          <div className="flex items-center justify-between border-t border-white/[0.06] px-6 py-3">
            <div className="flex items-center gap-2.5">
              <div className="flex h-7 w-7 items-center justify-center rounded-full bg-white/15">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="rgba(255,255,255,0.7)">
                  <circle cx="12" cy="8" r="4" />
                  <path d="M4 20c0-4 4-7 8-7s8 3 8 7" />
                </svg>
              </div>
              <span className="text-[12px] font-medium text-white/70">User</span>
            </div>
            <button className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-[8px] transition-colors duration-150 hover:bg-white/[0.06]">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-50">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
                <line x1="12" y1="2" x2="12" y2="12" />
              </svg>
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
