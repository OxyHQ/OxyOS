import { motion, AnimatePresence } from "framer-motion";
import { useLauncherStore } from "../../stores/launcherStore";

import browserIcon from "../../assets/icons/browser.svg";
import discordIcon from "../../assets/icons/discord.svg";
import spotifyIcon from "../../assets/icons/spotify.svg";
import youtubeIcon from "../../assets/icons/youtube.svg";
import mailIcon from "../../assets/icons/mail.svg";
import filesIcon from "../../assets/icons/files.svg";
import mapsIcon from "../../assets/icons/maps.svg";
import twitterIcon from "../../assets/icons/twitter.svg";
import facebookIcon from "../../assets/icons/facebook.svg";
import calendarIcon from "../../assets/icons/calendar.svg";
import photosIcon from "../../assets/icons/photos.svg";
import cameraIcon from "../../assets/icons/camera.svg";
import settingsIcon from "../../assets/icons/settings.svg";
import figmaIcon from "../../assets/icons/figma.svg";
import storeIcon from "../../assets/icons/store.svg";
import terminalIcon from "../../assets/icons/terminal.svg";
import messagesIcon from "../../assets/icons/messages.svg";
import calculatorIcon from "../../assets/icons/calculator.svg";
import clockIcon from "../../assets/icons/clock.svg";

interface AppEntry {
  name: string;
  icon: string;
}

const apps: AppEntry[] = [
  { name: "Chrome", icon: browserIcon },
  { name: "Discord", icon: discordIcon },
  { name: "Spotify", icon: spotifyIcon },
  { name: "YouTube", icon: youtubeIcon },
  { name: "Gmail", icon: mailIcon },
  { name: "Files", icon: filesIcon },
  { name: "Maps", icon: mapsIcon },
  { name: "Twitter", icon: twitterIcon },
  { name: "Facebook", icon: facebookIcon },
  { name: "Calendar", icon: calendarIcon },
  { name: "Photos", icon: photosIcon },
  { name: "Camera", icon: cameraIcon },
  { name: "Settings", icon: settingsIcon },
  { name: "Figma", icon: figmaIcon },
  { name: "Store", icon: storeIcon },
  { name: "Terminal", icon: terminalIcon },
  { name: "Messages", icon: messagesIcon },
  { name: "Calculator", icon: calculatorIcon },
  { name: "Clock", icon: clockIcon },
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
                  <div className="flex h-[56px] w-[56px] items-center justify-center overflow-hidden rounded-full">
                    <img
                      src={app.icon}
                      alt={app.name}
                      className="h-[72px] w-[72px] object-contain"
                      draggable={false}
                    />
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
