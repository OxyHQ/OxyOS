import { motion } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { useSessionStore } from "../../stores/sessionStore";
import { invoke } from "../../lib/tauri";

interface QuickSettingsProps {
  onClose: () => void;
}

const sliderThumb =
  "[&::-webkit-slider-thumb]:h-[18px] [&::-webkit-slider-thumb]:w-[18px] [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-0 [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)] [&::-moz-range-thumb]:h-[18px] [&::-moz-range-thumb]:w-[18px] [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.4)]";

function ConnectivityTile({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex flex-col items-center justify-center gap-1.5 rounded-2xl p-3 transition-colors duration-150 ${
        active
          ? "bg-[#0a84ff]/30 text-white"
          : "bg-white/8 text-white/60 hover:bg-white/12"
      }`}
    >
      <div className="flex h-8 w-8 items-center justify-center">{icon}</div>
      <span className="text-[10px] font-medium leading-tight">{label}</span>
    </button>
  );
}

function ControlModule({
  children,
  className = "",
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={`rounded-2xl border border-white/8 bg-white/6 ${className}`}>
      {children}
    </div>
  );
}

function QuickSettings({ onClose }: QuickSettingsProps) {
  const {
    wifiEnabled,
    bluetoothEnabled,
    quickShareEnabled,
    dndEnabled,
    nightLightEnabled,
    volume,
    brightness,
    batteryLevel,
    isCharging,
    toggleWifi,
    toggleBluetooth,
    toggleQuickShare,
    toggleDnd,
    toggleNightLight,
    setVolume,
    setBrightness,
  } = useSystemStore();

  const batteryFill = Math.round((batteryLevel / 100) * 18);
  const batteryColor = isCharging ? "#30d158" : batteryLevel <= 20 ? "#ff453a" : "white";

  return (
    <>
      {/* Click-away backdrop */}
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, y: 12, scale: 0.96 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 12, scale: 0.96 }}
        transition={{ duration: 0.22, ease: [0.2, 0, 0, 1] }}
        className="fixed right-2 bottom-[64px] z-50 w-[340px] origin-bottom-right overflow-hidden rounded-[22px] border border-white/15 bg-white/10 shadow-[0_12px_48px_rgba(0,0,0,0.4),inset_0_0.5px_0_rgba(255,255,255,0.12)] backdrop-blur-[70px] backdrop-saturate-[200%]"
      >
        <div className="flex flex-col gap-2.5 p-3">
          {/* ── Connectivity grid (2×2) ── */}
          <ControlModule>
            <div className="grid grid-cols-4 gap-px p-1.5">
              <ConnectivityTile
                active={wifiEnabled}
                onClick={toggleWifi}
                label="Wi-Fi"
                icon={
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
                  </svg>
                }
              />
              <ConnectivityTile
                active={bluetoothEnabled}
                onClick={toggleBluetooth}
                label="Bluetooth"
                icon={
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
                  </svg>
                }
              />
              <ConnectivityTile
                active={quickShareEnabled}
                onClick={toggleQuickShare}
                label="Quick Share"
                icon={
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8" />
                    <polyline points="16 6 12 2 8 6" />
                    <line x1="12" y1="2" x2="12" y2="15" />
                  </svg>
                }
              />
              <ConnectivityTile
                active={false}
                onClick={() => {}}
                label="Hotspot"
                icon={
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M8.56 3.69a9 9 0 0 0-2.92 1.95" />
                    <path d="M3.69 8.56A9 9 0 0 0 3 12" />
                    <path d="M3.69 15.44a9 9 0 0 0 1.95 2.92" />
                    <path d="M8.56 20.31A9 9 0 0 0 12 21" />
                    <path d="M15.44 20.31a9 9 0 0 0 2.92-1.95" />
                    <path d="M20.31 15.44A9 9 0 0 0 21 12" />
                    <path d="M20.31 8.56a9 9 0 0 0-1.95-2.92" />
                    <path d="M15.44 3.69A9 9 0 0 0 12 3" />
                    <circle cx="12" cy="12" r="2" />
                  </svg>
                }
              />
            </div>
          </ControlModule>

          {/* ── Focus + Screen Mirroring row ── */}
          <div className="grid grid-cols-2 gap-2.5">
            <ControlModule className="p-3">
              <button
                onClick={toggleDnd}
                className="flex w-full items-center gap-2.5"
              >
                <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full ${dndEnabled ? "bg-[#bf5af2]" : "bg-white/10"}`}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="white">
                    <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
                  </svg>
                </div>
                <div className="min-w-0 text-left">
                  <p className="truncate text-[12px] font-semibold leading-tight text-white/90">Focus</p>
                  <p className="truncate text-[10px] leading-tight text-white/45">{dndEnabled ? "Do Not Disturb" : "Off"}</p>
                </div>
              </button>
            </ControlModule>
            <ControlModule className="p-3">
              <button className="flex w-full items-center gap-2.5">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white/10">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <rect x="2" y="3" width="20" height="14" rx="2" />
                    <line x1="8" y1="21" x2="16" y2="21" />
                    <line x1="12" y1="17" x2="12" y2="21" />
                  </svg>
                </div>
                <div className="min-w-0 text-left">
                  <p className="truncate text-[12px] font-semibold leading-tight text-white/90">Display</p>
                  <p className="truncate text-[10px] leading-tight text-white/45">Built-in</p>
                </div>
              </button>
            </ControlModule>
          </div>

          {/* ── Now Playing ── */}
          <ControlModule className="p-3.5">
            <div className="mb-1 text-[10px] font-semibold tracking-wider text-white/35 uppercase">Now Playing</div>
            <div className="flex items-center gap-3">
              <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-white/8">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="white" fillOpacity="0.4">
                  <path d="M12 3v10.55c-.59-.34-1.27-.55-2-.55C7.79 13 6 14.79 6 17s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" />
                </svg>
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] font-semibold leading-tight text-white/85">Not Playing</p>
                <p className="truncate text-[11px] leading-tight text-white/40">Music</p>
              </div>
              <div className="flex items-center gap-0.5">
                <button className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full text-white/40 transition-colors hover:bg-white/8 hover:text-white/70">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M6 6l8.5 6L6 18V6zM16 6v12h2V6h-2z" transform="scale(-1,1) translate(-24,0)" />
                  </svg>
                </button>
                <button className="flex h-9 w-9 cursor-pointer items-center justify-center rounded-full text-white/50 transition-colors hover:bg-white/8 hover:text-white/80">
                  <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M8 5v14l11-7z" />
                  </svg>
                </button>
                <button className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full text-white/40 transition-colors hover:bg-white/8 hover:text-white/70">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z" />
                  </svg>
                </button>
              </div>
            </div>
          </ControlModule>

          {/* ── Night Light + Stage Manager row ── */}
          <div className="grid grid-cols-2 gap-2.5">
            <ControlModule className="p-3">
              <button
                onClick={toggleNightLight}
                className="flex w-full items-center gap-2.5"
              >
                <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full ${nightLightEnabled ? "bg-[#ff9f0a]" : "bg-white/10"}`}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="white">
                    <circle cx="12" cy="12" r="5" />
                    <line x1="12" y1="1" x2="12" y2="3" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="12" y1="21" x2="12" y2="23" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="1" y1="12" x2="3" y2="12" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="21" y1="12" x2="23" y2="12" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" stroke="white" strokeWidth="2" strokeLinecap="round" />
                    <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" stroke="white" strokeWidth="2" strokeLinecap="round" />
                  </svg>
                </div>
                <div className="min-w-0 text-left">
                  <p className="truncate text-[12px] font-semibold leading-tight text-white/90">Night Light</p>
                  <p className="truncate text-[10px] leading-tight text-white/45">{nightLightEnabled ? "On" : "Off"}</p>
                </div>
              </button>
            </ControlModule>
            <ControlModule className="p-3">
              <button className="flex w-full items-center gap-2.5">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white/10">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <rect x="2" y="6" width="8" height="12" rx="1" />
                    <rect x="14" y="2" width="8" height="8" rx="1" />
                    <rect x="14" y="14" width="8" height="8" rx="1" />
                  </svg>
                </div>
                <div className="min-w-0 text-left">
                  <p className="truncate text-[12px] font-semibold leading-tight text-white/90">Stage Manager</p>
                  <p className="truncate text-[10px] leading-tight text-white/45">Off</p>
                </div>
              </button>
            </ControlModule>
          </div>

          {/* ── Brightness slider ── */}
          <ControlModule className="flex items-center gap-3 px-4 py-3">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.45" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <circle cx="12" cy="12" r="5" />
              <line x1="12" y1="1" x2="12" y2="3" />
              <line x1="12" y1="21" x2="12" y2="23" />
              <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
              <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
              <line x1="1" y1="12" x2="3" y2="12" />
              <line x1="21" y1="12" x2="23" y2="12" />
              <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
              <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
            </svg>
            <input
              type="range"
              min={0}
              max={100}
              value={brightness}
              onChange={(e) => {
                const val = Number(e.target.value);
                setBrightness(val);
                invoke("set_brightness", { level: val });
              }}
              className={`h-[6px] flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
              style={{ background: `linear-gradient(to right, #0a84ff ${brightness}%, rgba(255,255,255,0.12) ${brightness}%)` }}
            />
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.45" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <circle cx="12" cy="12" r="5" />
              <line x1="12" y1="1" x2="12" y2="3" />
              <line x1="12" y1="21" x2="12" y2="23" />
              <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
              <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
              <line x1="1" y1="12" x2="3" y2="12" />
              <line x1="21" y1="12" x2="23" y2="12" />
              <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
              <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
            </svg>
          </ControlModule>

          {/* ── Volume slider ── */}
          <ControlModule className="flex items-center gap-3 px-4 py-3">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.35" className="shrink-0">
              <path d="M7 9v6h4l5 5V4l-5 5H7z" />
            </svg>
            <input
              type="range"
              min={0}
              max={100}
              value={volume}
              onChange={(e) => {
                const val = Number(e.target.value);
                setVolume(val);
                invoke("set_volume", { level: val });
              }}
              className={`h-[6px] flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
              style={{ background: `linear-gradient(to right, #0a84ff ${volume}%, rgba(255,255,255,0.12) ${volume}%)` }}
            />
            <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.35" className="shrink-0">
              <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
            </svg>
          </ControlModule>
        </div>

        {/* ── Bottom bar ── */}
        <div className="flex items-center justify-between border-t border-white/8 px-4 py-2.5">
          <div className="flex items-center gap-2">
            <svg width="28" height="12" viewBox="0 0 30 14" fill="none" className="rotate-180">
              <rect x="0" y="0" width="25" height="14" rx="5" fill="white" opacity="0.2" />
              <rect x="0" y="0" width={batteryFill} height="14" rx="5" fill={batteryColor} opacity="0.85" />
              <rect x="26" y="4" width="3" height="6" rx="1.5" fill="white" opacity="0.3" />
            </svg>
            <span className="text-[11px] font-medium text-white/45">
              {batteryLevel}%{isCharging ? " Charging" : ""}
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <button
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
              aria-label="Settings"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
            </button>
            <button
              onClick={() => {
                invoke("power_action", { action: "lock" });
                useSessionStore.getState().logout();
              }}
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
              aria-label="Lock"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="11" width="18" height="11" rx="2" />
                <path d="M7 11V7a5 5 0 0 1 10 0v4" />
              </svg>
            </button>
            <button
              onClick={() => invoke("power_action", { action: "shutdown" })}
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[#ff453a]/20"
              aria-label="Power off"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.55" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
                <line x1="12" y1="2" x2="12" y2="12" />
              </svg>
            </button>
          </div>
        </div>
      </motion.div>
    </>
  );
}

export default QuickSettings;
