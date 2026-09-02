import { motion } from "framer-motion";
import { useShallow } from "zustand/react/shallow";
import { useSystemStore } from "../../stores/systemStore";
import { useSessionStore } from "../../stores/sessionStore";
import { invoke } from "../../lib/tauri";
import avatarDefault from "../../assets/avatar-default.png";

interface QuickSettingsProps {
  onClose: () => void;
}

import { glass, getBatteryVisuals, sliderThumb } from "../../lib/styles";

function ConnectivityPill({
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
      className={`${glass.quickSettings} flex flex-col items-center justify-center gap-1 px-2 py-2.5 transition-all duration-150 ${
        active
          ? "!bg-white/20 text-white"
          : "text-white/55 hover:bg-white/16"
      }`}
    >
      {icon}
      <span className="text-[9px] font-medium leading-none">{label}</span>
    </button>
  );
}

function Row({ children, delay = 0, className = "" }: { children: React.ReactNode; delay?: number; className?: string }) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.22, delay, ease: [0.2, 0, 0, 1] }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

function QuickSettings({ onClose }: QuickSettingsProps) {
  const {
    wifiEnabled,
    bluetoothEnabled,
    nightLightEnabled,
    volume,
    brightnessPresent,
    brightness,
    batteryPresent,
    batteryLevel,
    isCharging,
    setWifi,
    setBluetooth,
    setNightLight,
    setVolume,
    setBrightness,
  } = useSystemStore(useShallow((s) => ({
    wifiEnabled: s.wifiEnabled,
    bluetoothEnabled: s.bluetoothEnabled,
    nightLightEnabled: s.nightLightEnabled,
    volume: s.volume,
    brightnessPresent: s.brightnessPresent,
    brightness: s.brightness,
    batteryPresent: s.batteryPresent,
    batteryLevel: s.batteryLevel,
    isCharging: s.isCharging,
    setWifi: s.setWifi,
    setBluetooth: s.setBluetooth,
    setNightLight: s.setNightLight,
    setVolume: s.setVolume,
    setBrightness: s.setBrightness,
  })));

  const { fillWidth: batteryFill, fillColor: batteryColor } = getBatteryVisuals(batteryLevel, isCharging);
  const avatarUrl = useSessionStore((s) => s.avatarUrl);

  const toggleWifi = () => {
    const next = !wifiEnabled;
    setWifi(next);
    invoke("set_wifi", { enabled: next });
  };
  const toggleBluetooth = () => {
    const next = !bluetoothEnabled;
    setBluetooth(next);
    invoke("set_bluetooth", { enabled: next });
  };
  const toggleNightLight = () => {
    const next = !nightLightEnabled;
    setNightLight(next);
    invoke("set_night_light", { enabled: next });
  };

  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <div className="fixed right-2 bottom-[64px] z-50 flex w-[320px] origin-bottom-right flex-col gap-2">
        {/* Connectivity */}
        <Row delay={0} className="grid grid-cols-2 gap-2">
          <ConnectivityPill
            active={wifiEnabled}
            onClick={toggleWifi}
            label="Wi-Fi"
            icon={
              <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
              </svg>
            }
          />
          <ConnectivityPill
            active={bluetoothEnabled}
            onClick={toggleBluetooth}
            label="Bluetooth"
            icon={
              <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
              </svg>
            }
          />
        </Row>

        {/* Night Light */}
        <Row delay={0.06}>
          <button
            onClick={toggleNightLight}
            className={`${glass.quickSettings} flex w-full items-center gap-2.5 px-3.5 py-3 transition-colors`}
          >
            <div className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full ${nightLightEnabled ? "bg-[#ff9f0a]" : "bg-white/12"}`}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="white">
                <circle cx="12" cy="12" r="5" />
              </svg>
            </div>
            <div className="min-w-0 text-left">
              <p className="truncate text-[11px] font-semibold leading-tight text-white/90">Night Light</p>
              <p className="truncate text-[9px] leading-tight text-white/40">{nightLightEnabled ? "On" : "Off"}</p>
            </div>
          </button>
        </Row>

        {brightnessPresent && (
          <Row delay={0.09}>
            <div className={`${glass.quickSettings} flex items-center gap-3 px-4 py-3`}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.3" className="shrink-0">
                <circle cx="12" cy="12" r="5" />
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
                style={{ background: `linear-gradient(to right, white ${brightness}%, rgba(255,255,255,0.1) ${brightness}%)` }}
              />
              <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.55" className="shrink-0">
                <circle cx="12" cy="12" r="5" />
              </svg>
            </div>
          </Row>
        )}

        {/* Volume */}
        <Row delay={0.12}>
          <div className={`${glass.quickSettings} flex items-center gap-3 px-4 py-3`}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.3" className="shrink-0">
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
              style={{ background: `linear-gradient(to right, white ${volume}%, rgba(255,255,255,0.1) ${volume}%)` }}
            />
            <svg width="14" height="14" viewBox="0 0 24 24" fill="white" fillOpacity="0.55" className="shrink-0">
              <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
            </svg>
          </div>
        </Row>

        {/* Bottom bar */}
        <Row delay={0.15}>
          <div className={`${glass.quickSettings} flex items-center justify-between px-3.5 py-2`}>
            <div className="flex items-center gap-2.5">
              <img src={avatarUrl ?? avatarDefault} alt="User" className="h-6 w-6 rounded-full object-cover ring-1 ring-white/20" draggable={false} />
              {batteryPresent && (
                <div className="flex items-center gap-1.5">
                  <svg width="22" height="10" viewBox="0 0 30 14" fill="none" className="rotate-180">
                    <rect x="0" y="0" width="25" height="14" rx="5" fill="white" opacity="0.2" />
                    <rect x="0" y="0" width={batteryFill} height="14" rx="5" fill={batteryColor} opacity="0.85" />
                    <rect x="26" y="4" width="3" height="6" rx="1.5" fill="white" opacity="0.3" />
                  </svg>
                  <span className="text-[10px] font-medium text-white/40">{batteryLevel}%</span>
                </div>
              )}
            </div>
            <div className="flex items-center gap-0.5">
              <button className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-white/10" aria-label="Settings">
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.5" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                </svg>
              </button>
              <button
                onClick={() => { invoke("power_action", { action: "lock" }); useSessionStore.getState().lock(); }}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-white/10" aria-label="Lock"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.5" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" />
                </svg>
              </button>
              <button
                onClick={() => invoke("power_action", { action: "shutdown" })}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-[#ff453a]/20" aria-label="Power off"
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="white" strokeOpacity="0.5" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M18.36 6.64a9 9 0 1 1-12.73 0" /><line x1="12" y1="2" x2="12" y2="12" />
                </svg>
              </button>
            </div>
          </div>
        </Row>
      </div>
    </>
  );
}

export default QuickSettings;
