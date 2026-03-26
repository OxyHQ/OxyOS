import { motion } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { useSessionStore } from "../../stores/sessionStore";
import { invoke } from "../../lib/tauri";

interface QuickSettingsProps {
  onClose: () => void;
}

const sliderThumb =
  "[&::-webkit-slider-thumb]:h-5 [&::-webkit-slider-thumb]:w-5 [&::-webkit-slider-thumb]:cursor-pointer [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-2 [&::-webkit-slider-thumb]:border-[#0a84ff] [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.35)] [&::-moz-range-thumb]:h-5 [&::-moz-range-thumb]:w-5 [&::-moz-range-thumb]:cursor-pointer [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-2 [&::-moz-range-thumb]:border-[#0a84ff] [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_4px_rgba(0,0,0,0.35)]";

function ToggleTile({
  active,
  onClick,
  icon,
  label,
  subtitle,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
  subtitle?: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-3 rounded-2xl px-4 py-3.5 text-left transition-colors duration-150 ${
        active
          ? "bg-[#0a84ff]/30 text-white"
          : "bg-white/8 text-white/70 hover:bg-white/12"
      }`}
    >
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white/10">
        {icon}
      </div>
      <div className="min-w-0">
        <div className="truncate text-[13px] font-medium leading-tight">{label}</div>
        {subtitle && (
          <div className="truncate text-[11px] leading-tight opacity-60">{subtitle}</div>
        )}
      </div>
    </button>
  );
}

function QuickSettings({ onClose }: QuickSettingsProps) {
  const {
    wifiEnabled,
    bluetoothEnabled,
    airdropEnabled,
    dndEnabled,
    nightLightEnabled,
    volume,
    brightness,
    batteryLevel,
    isCharging,
    toggleWifi,
    toggleBluetooth,
    toggleAirdrop,
    toggleDnd,
    toggleNightLight,
    setVolume,
    setBrightness,
  } = useSystemStore();

  return (
    <>
      {/* Click-away backdrop */}
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, y: 10, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 10, scale: 0.97 }}
        transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
        className="fixed bottom-[64px] right-2 z-50 w-[360px] origin-bottom-right overflow-hidden rounded-[20px] border border-white/20 bg-white/12 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]"
      >
        {/* ── Connectivity tiles ── */}
        <div className="grid grid-cols-2 gap-2 p-4 pb-2">
          <ToggleTile
            active={wifiEnabled}
            onClick={toggleWifi}
            label="Wi-Fi"
            subtitle={wifiEnabled ? "Connected" : "Off"}
            icon={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
              </svg>
            }
          />
          <ToggleTile
            active={bluetoothEnabled}
            onClick={toggleBluetooth}
            label="Bluetooth"
            subtitle={bluetoothEnabled ? "On" : "Off"}
            icon={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
              </svg>
            }
          />
          <ToggleTile
            active={airdropEnabled}
            onClick={toggleAirdrop}
            label="AirDrop"
            subtitle={airdropEnabled ? "Everyone" : "Off"}
            icon={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 2L2 7l10 5 10-5-10-5z" />
                <path d="M2 17l10 5 10-5" />
                <path d="M2 12l10 5 10-5" />
              </svg>
            }
          />
          <ToggleTile
            active={dndEnabled}
            onClick={toggleDnd}
            label="Focus"
            subtitle={dndEnabled ? "Do Not Disturb" : "Off"}
            icon={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            }
          />
        </div>

        {/* ── Additional toggles row ── */}
        <div className="flex gap-2 px-4 pb-3">
          <ToggleTile
            active={nightLightEnabled}
            onClick={toggleNightLight}
            label="Night Light"
            icon={
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            }
          />
          <button className="flex flex-1 items-center gap-3 rounded-2xl bg-white/8 px-4 py-3.5 text-left text-white/70 transition-colors duration-150 hover:bg-white/12">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white/10">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="2" y="3" width="20" height="14" rx="2" />
                <line x1="8" y1="21" x2="16" y2="21" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            </div>
            <div className="truncate text-[13px] font-medium leading-tight">Screen Mirroring</div>
          </button>
        </div>

        {/* ── Now Playing ── */}
        <div className="mx-4 mb-3 rounded-2xl bg-white/8 p-3.5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-white/10">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="white" fillOpacity="0.6">
                <path d="M12 3v10.55c-.59-.34-1.27-.55-2-.55C7.79 13 6 14.79 6 17s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" />
              </svg>
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-[13px] font-medium leading-tight text-white/90">Not Playing</p>
              <p className="truncate text-[11px] leading-tight text-white/45">Music</p>
            </div>
            <div className="flex items-center gap-1">
              <button className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/50 transition-colors hover:bg-white/10 hover:text-white/80">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M6 6h12v12H6z" />
                </svg>
              </button>
              <button className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/50 transition-colors hover:bg-white/10 hover:text-white/80">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M8 5v14l11-7z" />
                </svg>
              </button>
              <button className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/50 transition-colors hover:bg-white/10 hover:text-white/80">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z" />
                </svg>
              </button>
            </div>
          </div>
        </div>

        {/* ── Sliders ── */}
        <div className="border-t border-white/10 px-5 pt-4 pb-3">
          {/* Display */}
          <div className="mb-1 text-[10px] font-semibold tracking-wider text-white/40 uppercase">Display</div>
          <div className="mb-3 flex items-center gap-4">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.5" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
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
              className={`h-2 flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
              style={{ background: `linear-gradient(to right, #0a84ff ${brightness}%, rgba(255,255,255,0.15) ${brightness}%)` }}
            />
          </div>

          {/* Sound */}
          <div className="mb-1 text-[10px] font-semibold tracking-wider text-white/40 uppercase">Sound</div>
          <div className="flex items-center gap-4">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="white" fillOpacity="0.5" className="shrink-0">
              <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
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
              className={`h-2 flex-1 cursor-pointer appearance-none rounded-full outline-none ${sliderThumb}`}
              style={{ background: `linear-gradient(to right, #0a84ff ${volume}%, rgba(255,255,255,0.15) ${volume}%)` }}
            />
          </div>
        </div>

        {/* ── Bottom bar ── */}
        <div className="flex items-center justify-between border-t border-white/10 px-4 py-3">
          <div className="flex items-center gap-2">
            <svg width="14" height="14" viewBox="0 0 30 14" fill="none" className="rotate-180">
              <rect x="0" y="0" width="25" height="14" rx="5" fill="white" opacity="0.25" />
              <rect x="0" y="0" width={Math.round((batteryLevel / 100) * 18)} height="14" rx="5" fill={isCharging ? "#30d158" : batteryLevel <= 20 ? "#ff453a" : "white"} opacity="0.9" />
              <rect x="26" y="4" width="3" height="6" rx="1.5" fill="white" opacity="0.4" />
            </svg>
            <span className="text-[12px] text-white/50">
              {batteryLevel}%{isCharging ? " — Charging" : ""}
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <button
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
              aria-label="Settings"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.6" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
            </button>
            <button
              onClick={() => {
                invoke("power_action", { action: "lock" });
                useSessionStore.getState().logout();
              }}
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-white/10"
              aria-label="Lock"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.6" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="11" width="18" height="11" rx="2" />
                <path d="M7 11V7a5 5 0 0 1 10 0v4" />
              </svg>
            </button>
            <button
              onClick={() => invoke("power_action", { action: "shutdown" })}
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[#ff453a]/20"
              aria-label="Power off"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.6" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
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
