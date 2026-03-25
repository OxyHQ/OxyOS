import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";

interface AppEntry {
  name: string;
  color: string;
  icon: React.ReactNode;
}

const apps: AppEntry[] = [
  { name: "Chrome", color: "#4285f4", icon: <svg width="30" height="30" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="10" stroke="white" strokeWidth="1.5" /><ellipse cx="12" cy="12" rx="4" ry="10" stroke="white" strokeWidth="1.5" /><line x1="2" y1="12" x2="22" y2="12" stroke="white" strokeWidth="1.5" /></svg> },
  { name: "Discord", color: "#5865f2", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M19.27 5.33C17.94 4.71 16.5 4.26 15 4a.09.09 0 0 0-.07.03c-.18.33-.39.76-.53 1.09a16.09 16.09 0 0 0-4.8 0c-.14-.34-.36-.76-.54-1.09-.01-.02-.04-.03-.07-.03-1.5.26-2.93.71-4.27 1.33-.01 0-.02.01-.03.02-2.72 4.07-3.47 8.03-3.1 11.95 0 .02.01.04.03.05 1.8 1.32 3.53 2.12 5.24 2.65.03.01.06 0 .07-.02.4-.55.76-1.13 1.07-1.74.02-.04 0-.08-.04-.09-.57-.22-1.11-.48-1.64-.78-.04-.02-.04-.08-.01-.11.11-.08.22-.17.33-.25.02-.02.05-.02.07-.01 3.44 1.57 7.15 1.57 10.55 0 .02-.01.05-.01.07.01.11.09.22.17.33.26.04.03.04.09-.01.11-.52.31-1.07.56-1.64.78-.04.01-.05.06-.04.09.32.61.68 1.19 1.07 1.74.03.01.05.02.07.02 1.72-.53 3.45-1.33 5.24-2.65.02-.01.03-.03.03-.05.44-4.53-.73-8.46-3.1-11.95-.01-.01-.02-.02-.04-.02zM8.52 14.91c-1.03 0-1.89-.95-1.89-2.12s.84-2.12 1.89-2.12c1.06 0 1.9.96 1.89 2.12 0 1.17-.84 2.12-1.89 2.12zm6.97 0c-1.03 0-1.89-.95-1.89-2.12s.84-2.12 1.89-2.12c1.06 0 1.9.96 1.89 2.12 0 1.17-.83 2.12-1.89 2.12z" /></svg> },
  { name: "Spotify", color: "#1db954", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm4.64 14.36c-.2.2-.51.2-.71 0-1.95-1.19-4.41-1.46-7.31-.8-.28.07-.55-.1-.62-.38-.07-.28.1-.55.38-.62 3.17-.72 5.89-.41 8.07.93.2.13.26.4.13.6l.06.27zm1.23-2.71c-.25.25-.62.25-.87 0-2.23-1.37-5.63-1.77-8.27-.97-.33.1-.68-.09-.78-.42-.1-.33.09-.68.42-.78 3.01-.91 6.75-.47 9.29 1.1.25.16.32.5.16.75l.05.32z" /></svg> },
  { name: "YouTube", color: "#ff0000", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M10 15l5.19-3L10 9v6m11.56-7.83c.13.47.22 1.1.28 1.9.07.8.1 1.49.1 2.09L22 12c0 2.19-.16 3.8-.44 4.83-.25.9-.83 1.48-1.73 1.73-.47.13-1.33.22-2.65.28-1.3.07-2.49.1-3.59.1L12 19c-4.19 0-6.8-.16-7.83-.44-.9-.25-1.48-.83-1.73-1.73-.13-.47-.22-1.1-.28-1.9-.07-.8-.1-1.49-.1-2.09L2 12c0-2.19.16-3.8.44-4.83.25-.9.83-1.48 1.73-1.73.47-.13 1.33-.22 2.65-.28 1.3-.07 2.49-.1 3.59-.1L12 5c4.19 0 6.8.16 7.83.44.9.25 1.48.83 1.73 1.73z" /></svg> },
  { name: "Gmail", color: "#ea4335", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4l-8 5-8-5V6l8 5 8-5v2z" /></svg> },
  { name: "Files", color: "#4285f4", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M4 4h6l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" /></svg> },
  { name: "Maps", color: "#34a853", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M12 2C8.13 2 5 5.13 5 9c0 5.25 7 13 7 13s7-7.75 7-13c0-3.87-3.13-7-7-7zm0 9.5a2.5 2.5 0 1 1 0-5 2.5 2.5 0 0 1 0 5z" /></svg> },
  { name: "Twitter", color: "#1da1f2", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M22.46 6c-.77.35-1.6.58-2.46.69.88-.53 1.56-1.37 1.88-2.38-.83.5-1.75.85-2.72 1.05C18.37 4.5 17.26 4 16 4c-2.35 0-4.27 1.92-4.27 4.29 0 .34.04.67.11.98C8.28 9.09 5.11 7.38 3 4.79c-.37.63-.58 1.37-.58 2.15 0 1.49.75 2.81 1.91 3.56-.71 0-1.37-.2-1.95-.5v.03c0 2.08 1.48 3.82 3.44 4.21a4.22 4.22 0 0 1-1.93.07 4.28 4.28 0 0 0 4 2.98 8.521 8.521 0 0 1-5.33 1.84c-.34 0-.68-.02-1.02-.06C3.44 20.29 5.7 21 8.12 21 16 21 20.33 14.46 20.33 8.79c0-.19 0-.37-.01-.56.84-.6 1.56-1.36 2.14-2.23z" /></svg> },
  { name: "Facebook", color: "#1877f2", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M13.397 20.997v-8.196h2.765l.411-3.209h-3.176V7.548c0-.926.258-1.56 1.587-1.56h1.684V3.127A22.336 22.336 0 0 0 14.201 3c-2.444 0-4.122 1.492-4.122 4.231v2.355H7.332v3.209h2.753v8.202h3.312z" /></svg> },
  { name: "Reddit", color: "#ff4500", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><circle cx="12" cy="12" r="10" /><circle cx="8.5" cy="12.5" r="1.5" fill="#ff4500" /><circle cx="15.5" cy="12.5" r="1.5" fill="#ff4500" /></svg> },
  { name: "Drive", color: "#4285f4", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M7.71 3.5L1.15 15l2.86 4.97L10.57 8.47 7.71 3.5zm1.43 0l6.86 11.94H2.58l2.86-4.97L7.71 3.5h1.43zm7.72 11.94L12 3.5h5.72l4.86 8.47-2.86 4.97-2.86-1.5z" /></svg> },
  { name: "Settings", color: "#5f6368", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.49.49 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54A.48.48 0 0013.93 2h-3.86a.48.48 0 00-.48.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.71 8.81a.48.48 0 00.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.26.41.48.41h3.86c.22 0 .43-.17.48-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1115.6 12 3.6 3.6 0 0112 15.6z" /></svg> },
  { name: "Figma", color: "#a259ff", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M12 2H8.5A3.5 3.5 0 0 0 5 5.5 3.5 3.5 0 0 0 8.5 9H12V2zM12 9H8.5A3.5 3.5 0 0 0 5 12.5 3.5 3.5 0 0 0 8.5 16H12V9zM12 16H8.5a3.5 3.5 0 1 0 3.5 3.5V16zM12 2h3.5A3.5 3.5 0 0 1 19 5.5 3.5 3.5 0 0 1 15.5 9H12V2zM19 12.5a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0z" /></svg> },
  { name: "Keep", color: "#fbbc04", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M9 21c0 .55.45 1 1 1h4c.55 0 1-.45 1-1v-1H9v1zm3-19C8.14 2 5 5.14 5 9c0 2.38 1.19 4.47 3 5.74V17c0 .55.45 1 1 1h6c.55 0 1-.45 1-1v-2.26c1.81-1.27 3-3.36 3-5.74 0-3.86-3.14-7-7-7z" /></svg> },
  { name: "Terminal", color: "#202124", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="none"><path d="M7 8l4 4-4 4" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" /><line x1="13" y1="16" x2="17" y2="16" stroke="white" strokeWidth="2" strokeLinecap="round" /></svg> },
  { name: "Messages", color: "#1a73e8", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M20 2H4c-1.1 0-2 .9-2 2v18l4-4h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2z" /></svg> },
  { name: "Photos", color: "#fbbc04", icon: <svg width="28" height="28" viewBox="0 0 24 24"><circle cx="12" cy="10" r="3.5" fill="#ea4335" /><circle cx="8" cy="14.5" r="3.5" fill="#4285f4" /><circle cx="16" cy="14.5" r="3.5" fill="#34a853" /><circle cx="12" cy="12.5" r="2" fill="white" /></svg> },
  { name: "Camera", color: "#5f6368", icon: <svg width="28" height="28" viewBox="0 0 24 24" fill="white"><path d="M4 7h3l2-3h6l2 3h3a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z" /><circle cx="12" cy="13" r="3.5" fill="rgba(0,0,0,0.2)" /></svg> },
];

export default function AppLauncher() {
  const isOpen = useLauncherStore((s) => s.isOpen);
  const close = useLauncherStore((s) => s.close);

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          {/* Full-screen blurred backdrop — macOS Launchpad style */}
          <motion.div
            key="launcher-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            className="fixed inset-0 z-35 bg-black/50 backdrop-blur-[32px]"
            onClick={close}
          />

          {/* App grid — centered, no panel, just floating icons */}
          <motion.div
            key="launcher-grid"
            initial={{ opacity: 0, scale: 0.92 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
            className="fixed inset-x-0 top-0 bottom-[56px] z-40 flex flex-col items-center select-none"
          >
            {/* Search bar — top center */}
            <div className="mt-[6vh] w-[420px]">
              <div className="flex h-[40px] items-center gap-2.5 rounded-full bg-white/10 px-4">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-40">
                  <circle cx="11" cy="11" r="7" />
                  <path d="M21 21l-4.35-4.35" />
                </svg>
                <input
                  type="text"
                  placeholder="Search"
                  className="min-w-0 flex-1 bg-transparent text-[14px] text-white placeholder-white/35 outline-none"
                />
              </div>
            </div>

            {/* Apps — centered grid */}
            <div className="mt-[5vh] grid grid-cols-6 gap-x-10 gap-y-6">
              {apps.map((app) => (
                <button
                  key={app.name}
                  type="button"
                  className="flex cursor-pointer flex-col items-center gap-2 border-none bg-transparent p-0 transition-transform duration-100 active:scale-90"
                >
                  <div
                    className="flex h-[56px] w-[56px] items-center justify-center rounded-full shadow-[0_2px_8px_rgba(0,0,0,0.3)]"
                    style={{ backgroundColor: app.color }}
                  >
                    {app.icon}
                  </div>
                  <span className="max-w-[72px] truncate text-center text-[11px] font-medium leading-tight text-white/80 drop-shadow-[0_1px_2px_rgba(0,0,0,0.5)]">
                    {app.name}
                  </span>
                </button>
              ))}
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
