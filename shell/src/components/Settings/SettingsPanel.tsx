import { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useSettingsStore, type SettingsSection } from "../../stores/settingsStore";
import { useSystemStore } from "../../stores/systemStore";
import { glass, sliderThumb } from "../../lib/styles";

const sections: { name: SettingsSection; icon: React.ReactNode }[] = [
  {
    name: "Wi-Fi",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
        <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
      </svg>
    ),
  },
  {
    name: "Bluetooth",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
        <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
      </svg>
    ),
  },
  {
    name: "Display",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <line x1="8" y1="21" x2="16" y2="21" />
        <line x1="12" y1="17" x2="12" y2="21" />
      </svg>
    ),
  },
  {
    name: "Sound",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
        <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
      </svg>
    ),
  },
  {
    name: "Wallpaper",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="3" y="3" width="18" height="18" rx="2" />
        <circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" stroke="none" />
        <path d="M21 15l-5-5L5 21" />
      </svg>
    ),
  },
  {
    name: "About",
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z" />
      </svg>
    ),
  },
];

interface WifiNetwork {
  name: string;
  strength: number;
  secured: boolean;
}

const fakeNetworks: WifiNetwork[] = [
  { name: "Home WiFi", strength: 3, secured: true },
  { name: "Office 5G", strength: 3, secured: true },
  { name: "Guest Network", strength: 2, secured: false },
  { name: "Coffee Shop", strength: 1, secured: false },
];

function SignalIcon({ strength }: { strength: number }) {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
      <rect x="1" y="11" width="3" height="4" rx="0.5" fill={strength >= 1 ? "white" : "rgba(255,255,255,0.2)"} />
      <rect x="5" y="8" width="3" height="7" rx="0.5" fill={strength >= 2 ? "white" : "rgba(255,255,255,0.2)"} />
      <rect x="9" y="4" width="3" height="11" rx="0.5" fill={strength >= 3 ? "white" : "rgba(255,255,255,0.2)"} />
      <rect x="13" y="1" width="3" height="14" rx="0.5" fill={strength >= 4 ? "white" : "rgba(255,255,255,0.2)"} />
    </svg>
  );
}

function Toggle({ enabled, onToggle }: { enabled: boolean; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      className={`relative h-[26px] w-[46px] shrink-0 cursor-pointer rounded-full transition-colors duration-200 ${
        enabled ? "bg-[#30d158]" : "bg-white/20"
      }`}
    >
      <div
        className={`absolute top-[3px] h-5 w-5 rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.3)] transition-transform duration-200 ${
          enabled ? "left-[23px]" : "left-[3px]"
        }`}
      />
    </button>
  );
}

function WifiSection() {
  const { wifiEnabled, toggleWifi } = useSystemStore();
  const [connectedNetwork, setConnectedNetwork] = useState<string>("Home WiFi");

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-[14px] font-semibold text-white/90">Wi-Fi</h3>
          <p className="text-[12px] text-white/40">
            {wifiEnabled ? `Connected to ${connectedNetwork}` : "Disabled"}
          </p>
        </div>
        <Toggle enabled={wifiEnabled} onToggle={toggleWifi} />
      </div>

      {wifiEnabled && (
        <div className="flex flex-col gap-1">
          <p className="mb-1 text-[11px] font-medium tracking-wide text-white/50 uppercase">
            Available Networks
          </p>
          {fakeNetworks.map((network) => (
            <button
              key={network.name}
              onClick={() => setConnectedNetwork(network.name)}
              className={`flex w-full cursor-pointer items-center justify-between rounded-xl px-3 py-2.5 transition-colors ${
                connectedNetwork === network.name
                  ? "bg-white/15"
                  : "hover:bg-white/8"
              }`}
            >
              <div className="flex items-center gap-3">
                <SignalIcon strength={network.strength} />
                <span className="text-[13px] text-white/80">{network.name}</span>
                {network.secured && (
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="white" fillOpacity="0.35">
                    <path d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z" />
                  </svg>
                )}
              </div>
              {connectedNetwork === network.name && (
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#30d158" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function BluetoothSection() {
  const { bluetoothEnabled, toggleBluetooth } = useSystemStore();

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-[14px] font-semibold text-white/90">Bluetooth</h3>
          <p className="text-[12px] text-white/40">
            {bluetoothEnabled ? "On — Discoverable" : "Disabled"}
          </p>
        </div>
        <Toggle enabled={bluetoothEnabled} onToggle={toggleBluetooth} />
      </div>

      {bluetoothEnabled && (
        <div className="flex flex-col gap-1">
          <p className="mb-1 text-[11px] font-medium tracking-wide text-white/50 uppercase">
            Nearby Devices
          </p>
          <div className="flex flex-col items-center justify-center rounded-xl bg-white/5 py-8">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="white" fillOpacity="0.2">
              <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
            </svg>
            <p className="mt-2 text-[12px] text-white/35">Searching for devices...</p>
          </div>
        </div>
      )}
    </div>
  );
}

function DisplaySection() {
  const { brightness, setBrightness, nightLightEnabled, toggleNightLight } = useSystemStore();

  return (
    <div className="flex flex-col gap-5">
      <div>
        <h3 className="text-[14px] font-semibold text-white/90">Display</h3>
        <p className="text-[12px] text-white/40">Built-in Display</p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-[12px] font-medium text-white/60">Brightness</span>
          <span className="text-[12px] text-white/40">{brightness}%</span>
        </div>
        <div className="flex items-center gap-3">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.3" className="shrink-0">
            <circle cx="12" cy="12" r="5" />
          </svg>
          <input
            type="range"
            min={0}
            max={100}
            value={brightness}
            onChange={(e) => setBrightness(Number(e.target.value))}
            className={`h-[6px] flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
            style={{
              background: `linear-gradient(to right, white ${brightness}%, rgba(255,255,255,0.1) ${brightness}%)`,
            }}
          />
          <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.55" className="shrink-0">
            <circle cx="12" cy="12" r="5" />
            <line x1="12" y1="1" x2="12" y2="3" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="12" y1="21" x2="12" y2="23" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="1" y1="12" x2="3" y2="12" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="21" y1="12" x2="23" y2="12" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
            <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" />
          </svg>
        </div>
      </div>

      <div className="flex items-center justify-between rounded-xl bg-white/5 px-4 py-3">
        <div className="flex items-center gap-3">
          <div className={`flex h-8 w-8 items-center justify-center rounded-full ${nightLightEnabled ? "bg-[#ff9f0a]" : "bg-white/12"}`}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="white">
              <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
            </svg>
          </div>
          <div>
            <p className="text-[13px] font-medium text-white/80">Night Light</p>
            <p className="text-[11px] text-white/35">Reduces blue light</p>
          </div>
        </div>
        <Toggle enabled={nightLightEnabled} onToggle={toggleNightLight} />
      </div>

      <div className="flex flex-col gap-2 rounded-xl bg-white/5 px-4 py-3">
        <div className="flex items-center justify-between">
          <span className="text-[13px] font-medium text-white/80">Resolution</span>
          <span className="text-[12px] text-white/40">1920 x 1080</span>
        </div>
        <div className="flex items-center justify-between">
          <span className="text-[13px] font-medium text-white/80">Refresh Rate</span>
          <span className="text-[12px] text-white/40">60 Hz</span>
        </div>
      </div>
    </div>
  );
}

function SoundSection() {
  const { volume, setVolume } = useSystemStore();

  return (
    <div className="flex flex-col gap-5">
      <div>
        <h3 className="text-[14px] font-semibold text-white/90">Sound</h3>
        <p className="text-[12px] text-white/40">Output: Built-in Speakers</p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-[12px] font-medium text-white/60">Volume</span>
          <span className="text-[12px] text-white/40">{volume}%</span>
        </div>
        <div className="flex items-center gap-3">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.3" className="shrink-0">
            <path d="M7 9v6h4l5 5V4l-5 5H7z" />
          </svg>
          <input
            type="range"
            min={0}
            max={100}
            value={volume}
            onChange={(e) => setVolume(Number(e.target.value))}
            className={`h-[6px] flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
            style={{
              background: `linear-gradient(to right, white ${volume}%, rgba(255,255,255,0.1) ${volume}%)`,
            }}
          />
          <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.55" className="shrink-0">
            <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
          </svg>
        </div>
      </div>

      <div className="flex flex-col gap-1">
        <p className="mb-1 text-[11px] font-medium tracking-wide text-white/50 uppercase">
          Output Device
        </p>
        {["Built-in Speakers", "Headphones"].map((device) => (
          <div
            key={device}
            className={`flex items-center justify-between rounded-xl px-3 py-2.5 ${
              device === "Built-in Speakers" ? "bg-white/10" : "bg-white/5"
            }`}
          >
            <div className="flex items-center gap-3">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.5">
                {device === "Built-in Speakers" ? (
                  <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
                ) : (
                  <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3zM5 10a1 1 0 0 0-2 0 9 9 0 0 0 8 8.94V22h-2a1 1 0 0 0 0 2h6a1 1 0 0 0 0-2h-2v-3.06A9 9 0 0 0 21 10a1 1 0 0 0-2 0 7 7 0 0 1-14 0z" />
                )}
              </svg>
              <span className="text-[13px] text-white/80">{device}</span>
            </div>
            {device === "Built-in Speakers" && (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#30d158" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

const wallpaperSwatches = [
  { id: "default", color: "linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%)", label: "Default" },
  { id: "ocean", color: "linear-gradient(135deg, #0077b6 0%, #023e8a 50%, #03045e 100%)", label: "Ocean" },
  { id: "sunset", color: "linear-gradient(135deg, #f72585 0%, #7209b7 50%, #3a0ca3 100%)", label: "Sunset" },
  { id: "forest", color: "linear-gradient(135deg, #2d6a4f 0%, #1b4332 50%, #081c15 100%)", label: "Forest" },
];

function WallpaperSection() {
  const { wallpaper, setWallpaper } = useSystemStore();

  function applyWallpaper(id: string) {
    setWallpaper(id);
    const swatch = wallpaperSwatches.find((s) => s.id === id);
    if (swatch) {
      document.documentElement.style.setProperty("--wallpaper-bg", swatch.color);
      const desktopEl = document.querySelector("[data-desktop-bg]") as HTMLElement | null;
      if (desktopEl) {
        desktopEl.style.background = swatch.color;
      }
    }
  }

  return (
    <div className="flex flex-col gap-5">
      <div>
        <h3 className="text-[14px] font-semibold text-white/90">Wallpaper</h3>
        <p className="text-[12px] text-white/40">Choose a background for your desktop</p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        {wallpaperSwatches.map((swatch) => (
          <button
            key={swatch.id}
            onClick={() => applyWallpaper(swatch.id)}
            className={`group relative cursor-pointer overflow-hidden rounded-2xl border-2 transition-all duration-200 ${
              wallpaper === swatch.id
                ? "border-[#0a84ff] shadow-[0_0_0_1px_#0a84ff]"
                : "border-white/10 hover:border-white/25"
            }`}
          >
            <div
              className="h-24 w-full"
              style={{ background: swatch.color }}
            />
            <div className="flex items-center justify-between bg-white/5 px-3 py-2">
              <span className="text-[12px] font-medium text-white/70">{swatch.label}</span>
              {wallpaper === swatch.id && (
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#0a84ff" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              )}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}

function AboutSection() {
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col items-center gap-3 py-4">
        <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-white/10">
          <svg width="32" height="32" viewBox="0 0 24 24" fill="white" fillOpacity="0.8">
            <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z" />
          </svg>
        </div>
        <div className="text-center">
          <h3 className="text-[18px] font-bold text-white/90">OxyOS</h3>
          <p className="text-[13px] text-white/50">Version 1.0</p>
        </div>
      </div>

      <div className="flex flex-col gap-1 rounded-xl bg-white/5 px-4 py-3">
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">System</span>
          <span className="text-[13px] font-medium text-white/80">OxyOS 1.0</span>
        </div>
        <div className="h-px bg-white/5" />
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">Build</span>
          <span className="text-[13px] font-medium text-white/80">2026.03.26</span>
        </div>
        <div className="h-px bg-white/5" />
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">Kernel</span>
          <span className="text-[13px] font-medium text-white/80">Linux 6.6</span>
        </div>
        <div className="h-px bg-white/5" />
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">Architecture</span>
          <span className="text-[13px] font-medium text-white/80">amd64</span>
        </div>
        <div className="h-px bg-white/5" />
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">Shell</span>
          <span className="text-[13px] font-medium text-white/80">OxyOS Shell 1.0</span>
        </div>
        <div className="h-px bg-white/5" />
        <div className="flex items-center justify-between py-1.5">
          <span className="text-[13px] text-white/60">Desktop</span>
          <span className="text-[13px] font-medium text-white/80">Liquid Glass</span>
        </div>
      </div>

      <div className="text-center">
        <p className="text-[11px] text-white/30">
          Designed by Oxy. Built with care.
        </p>
      </div>
    </div>
  );
}

const sectionComponents: Record<SettingsSection, React.FC> = {
  "Wi-Fi": WifiSection,
  Bluetooth: BluetoothSection,
  Display: DisplaySection,
  Sound: SoundSection,
  Wallpaper: WallpaperSection,
  About: AboutSection,
};

export default function SettingsPanel() {
  const { isOpen, close, activeSection, setActiveSection } = useSettingsStore();
  const ActiveComponent = sectionComponents[activeSection] ?? WifiSection;

  return (
    <AnimatePresence>
      {isOpen && (
        <>
          <motion.div
            className="fixed inset-0 z-50 bg-black/30"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={close}
          />

          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            transition={{ duration: 0.25, ease: [0.2, 0, 0, 1] }}
            className={`${glass.panel} fixed top-1/2 left-1/2 z-50 flex h-[500px] w-[700px] -translate-x-1/2 -translate-y-1/2 overflow-hidden`}
          >
            {/* Sidebar */}
            <div className="flex w-[200px] shrink-0 flex-col border-r border-white/10 py-3">
              <div className="mb-3 px-5">
                <h2 className="text-[15px] font-bold text-white/90">Settings</h2>
              </div>

              <nav className="flex flex-1 flex-col gap-0.5 px-2">
                {sections.map((section) => (
                  <button
                    key={section.name}
                    onClick={() => setActiveSection(section.name)}
                    className={`flex w-full cursor-pointer items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors duration-150 ${
                      activeSection === section.name
                        ? "bg-white/15 text-white"
                        : "text-white/60 hover:bg-white/8 hover:text-white/80"
                    }`}
                  >
                    <span className="shrink-0 opacity-80">{section.icon}</span>
                    <span className="text-[13px] font-medium">{section.name}</span>
                  </button>
                ))}
              </nav>
            </div>

            {/* Content */}
            <div className="relative flex-1 overflow-y-auto px-6 py-5 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
              {/* Close button */}
              <button
                onClick={close}
                className="absolute top-3 right-3 flex h-7 w-7 cursor-pointer items-center justify-center rounded-full bg-white/10 text-white/50 transition-colors hover:bg-white/20 hover:text-white/80"
                aria-label="Close settings"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                  <path d="M1 1l8 8M9 1l-8 8" />
                </svg>
              </button>

              <ActiveComponent />
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
