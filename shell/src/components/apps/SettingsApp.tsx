import { useState } from "react";
import { motion } from "framer-motion";
import { useSystemStore } from "../../stores/systemStore";
import { invoke } from "../../lib/tauri";
import { sliderThumb } from "../../lib/styles";

interface SettingsAppProps {
  onClose: () => void;
}

type Section = "wifi" | "bluetooth" | "sound" | "display" | "about";

const sections: { id: Section; label: string; icon: React.ReactNode }[] = [
  {
    id: "wifi",
    label: "Wi-Fi",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
        <path d="M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3a4.237 4.237 0 0 0-6 0zm-4-4l2 2a7.074 7.074 0 0 1 10 0l2-2C15.14 9.14 8.87 9.14 5 13z" />
      </svg>
    ),
  },
  {
    id: "bluetooth",
    label: "Bluetooth",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
        <path d="M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z" />
      </svg>
    ),
  },
  {
    id: "sound",
    label: "Sound",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
        <path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8.5v7a4.47 4.47 0 0 0 2.5-3.5zM14 3.23v2.06a7.007 7.007 0 0 1 0 13.42v2.06A9.005 9.005 0 0 0 14 3.23z" />
      </svg>
    ),
  },
  {
    id: "display",
    label: "Display",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2" />
        <line x1="8" y1="21" x2="16" y2="21" />
        <line x1="12" y1="17" x2="12" y2="21" />
      </svg>
    ),
  },
  {
    id: "about",
    label: "About",
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z" />
      </svg>
    ),
  },
];

function Toggle({ enabled, onToggle }: { enabled: boolean; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      className={`relative h-[26px] w-[46px] shrink-0 cursor-pointer rounded-full transition-colors duration-200 ${
        enabled ? "bg-[#1a73e8]" : "bg-[#dadce0]"
      }`}
    >
      <span
        className={`absolute top-[3px] left-[3px] h-[20px] w-[20px] rounded-full bg-white shadow-sm transition-transform duration-200 ${
          enabled ? "translate-x-[20px]" : ""
        }`}
      />
    </button>
  );
}

function SettingsSlider({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-[14px] font-medium text-[#202124]">{label}</span>
        <span className="text-[13px] text-[#5f6368]">{value}%</span>
      </div>
      <input
        type="range"
        min={0}
        max={100}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className={`h-[6px] w-full cursor-pointer appearance-none rounded-full outline-none ${sliderThumb} [&::-webkit-slider-thumb]:bg-[#1a73e8] [&::-moz-range-thumb]:bg-[#1a73e8]`}
        style={{
          background: `linear-gradient(to right, #1a73e8 ${value}%, #dadce0 ${value}%)`,
        }}
      />
    </div>
  );
}

function WifiSection() {
  const wifiEnabled = useSystemStore((s) => s.wifiEnabled);
  const toggleWifi = useSystemStore((s) => s.toggleWifi);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-[16px] font-medium text-[#202124]">Wi-Fi</h3>
          <p className="mt-1 text-[13px] text-[#5f6368]">
            {wifiEnabled ? "Connected" : "Disconnected"}
          </p>
        </div>
        <Toggle enabled={wifiEnabled} onToggle={toggleWifi} />
      </div>
      {wifiEnabled && (
        <div className="rounded-[12px] border border-[#e8eaed] p-4">
          <p className="text-[13px] font-medium text-[#202124]">OxyNetwork</p>
          <p className="mt-0.5 text-[12px] text-[#5f6368]">Connected, secured</p>
        </div>
      )}
    </div>
  );
}

function BluetoothSection() {
  const bluetoothEnabled = useSystemStore((s) => s.bluetoothEnabled);
  const toggleBluetooth = useSystemStore((s) => s.toggleBluetooth);

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-[16px] font-medium text-[#202124]">Bluetooth</h3>
          <p className="mt-1 text-[13px] text-[#5f6368]">
            {bluetoothEnabled ? "On — Discoverable" : "Off"}
          </p>
        </div>
        <Toggle enabled={bluetoothEnabled} onToggle={toggleBluetooth} />
      </div>
      {bluetoothEnabled && (
        <div className="rounded-[12px] border border-[#e8eaed] p-4">
          <p className="text-[13px] text-[#5f6368]">No devices paired</p>
        </div>
      )}
    </div>
  );
}

function SoundSection() {
  const volume = useSystemStore((s) => s.volume);
  const setVolume = useSystemStore((s) => s.setVolume);

  return (
    <div className="flex flex-col gap-6">
      <h3 className="text-[16px] font-medium text-[#202124]">Sound</h3>
      <SettingsSlider
        label="Volume"
        value={volume}
        onChange={(v) => {
          setVolume(v);
          invoke("set_volume", { level: v });
        }}
      />
      <div className="rounded-[12px] border border-[#e8eaed] p-4">
        <p className="text-[13px] font-medium text-[#202124]">Output device</p>
        <p className="mt-0.5 text-[12px] text-[#5f6368]">Built-in speakers</p>
      </div>
    </div>
  );
}

function DisplaySection() {
  const brightness = useSystemStore((s) => s.brightness);
  const setBrightness = useSystemStore((s) => s.setBrightness);

  return (
    <div className="flex flex-col gap-6">
      <h3 className="text-[16px] font-medium text-[#202124]">Display</h3>
      <SettingsSlider
        label="Brightness"
        value={brightness}
        onChange={(v) => {
          setBrightness(v);
          invoke("set_brightness", { level: v });
        }}
      />
      <div className="rounded-[12px] border border-[#e8eaed] p-4">
        <p className="text-[13px] font-medium text-[#202124]">Built-in display</p>
        <p className="mt-0.5 text-[12px] text-[#5f6368]">1920 x 1080</p>
      </div>
    </div>
  );
}

function AboutSection() {
  return (
    <div className="flex flex-col gap-6">
      <h3 className="text-[16px] font-medium text-[#202124]">About OxyOS</h3>
      <div className="flex flex-col gap-4 rounded-[12px] border border-[#e8eaed] p-4">
        <div>
          <p className="text-[12px] text-[#5f6368]">Version</p>
          <p className="text-[14px] font-medium text-[#202124]">OxyOS 1.3</p>
        </div>
        <div>
          <p className="text-[12px] text-[#5f6368]">Architecture</p>
          <p className="text-[14px] font-medium text-[#202124]">x86_64 (amd64)</p>
        </div>
        <div>
          <p className="text-[12px] text-[#5f6368]">Kernel</p>
          <p className="text-[14px] font-medium text-[#202124]">Linux</p>
        </div>
        <div>
          <p className="text-[12px] text-[#5f6368]">Desktop</p>
          <p className="text-[14px] font-medium text-[#202124]">OxyOS Shell</p>
        </div>
      </div>
    </div>
  );
}

const sectionComponents: Record<Section, React.FC> = {
  wifi: WifiSection,
  bluetooth: BluetoothSection,
  sound: SoundSection,
  display: DisplaySection,
  about: AboutSection,
};

function SettingsApp({ onClose }: SettingsAppProps) {
  const [activeSection, setActiveSection] = useState<Section>("wifi");
  const ActiveContent = sectionComponents[activeSection];

  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <motion.div
        initial={{ opacity: 0, scale: 0.95 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.95 }}
        transition={{ duration: 0.2, ease: [0.2, 0, 0, 1] }}
        className="fixed top-1/2 left-1/2 z-50 flex h-[500px] w-[700px] -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-[16px] bg-white shadow-[0_8px_40px_rgba(0,0,0,0.18)]"
      >
        {/* Sidebar */}
        <div className="flex w-[200px] shrink-0 flex-col bg-[#f8f9fa]">
          <div className="flex items-center justify-between px-5 pt-5 pb-4">
            <h1 className="text-[16px] font-semibold text-[#202124]">Settings</h1>
            <button
              onClick={onClose}
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-[#5f6368] transition-colors hover:bg-[#e8eaed]"
              aria-label="Close"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>

          <nav className="flex flex-col gap-0.5 px-2">
            {sections.map((section) => (
              <button
                key={section.id}
                onClick={() => setActiveSection(section.id)}
                className={`flex cursor-pointer items-center gap-3 rounded-[10px] px-3 py-2.5 text-left transition-colors ${
                  activeSection === section.id
                    ? "bg-[#e8f0fe] text-[#1a73e8]"
                    : "text-[#202124] hover:bg-[#e8eaed]"
                }`}
              >
                <span className="shrink-0">{section.icon}</span>
                <span className="text-[13px] font-medium">{section.label}</span>
              </button>
            ))}
          </nav>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-8">
          <ActiveContent />
        </div>
      </motion.div>
    </>
  );
}

export default SettingsApp;
