import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { BloomProvider } from "@oxy.so/bloom/provider";
import { Shell, type ShellCommand } from "../../../apps/shell/src/Shell";
import "./styles.css";
import { previewBluetooth, previewNetworks, previewSearch, previewSnapshot, previewSound, previewWallpaper } from "./fixtures";
import type {
  ShellActions,
  ShellControls,
  ShellPowerActions,
} from "../../../apps/shell/src/model";
import { isProtected, type NetworkState } from "../../../apps/shell/src/network";
import type { SoundState } from "../../../apps/shell/src/sound";
import { bluetoothSummary, type BluetoothState } from "../../../apps/shell/src/bluetooth";
import type { SheetText } from "../../../apps/shell/src/sheet";
import { shellMessages } from "../../../apps/shell/src/messages";
import { inPlace, type ShellPlatform } from "../../../apps/shell/src/platform";
import { ActivityMonitor } from "../../../apps/activity-monitor/src/ActivityMonitor";
import { previewActivity } from "./activity-fixture";

/**
 * The preview's locale (docs/adr/0040-language-and-region.mdx): the browser's, or the one `?lang=`
 * asks for (`en`, `es`, or any tag, such as `en-GB` or `es-MX-u-hc-h12`). As the session makes
 * its own, a language OxyOS does not speak is English, in the tag's region.
 */
function previewLocale(): string {
  const asked = new URLSearchParams(window.location.search).get("lang");
  let tag = navigator.language || "en-US";
  if (asked) {
    try {
      tag = Intl.getCanonicalLocales(asked)[0] ?? tag;
    } catch {
      console.warn("[shell preview] not a language tag:", asked);
    }
  }
  const chosen = new Intl.Locale(tag);
  if (chosen.language === "en" || chosen.language === "es") return tag;
  return new Intl.Locale("en", { region: chosen.region ?? "US" }).toString();
}
const locale = previewLocale();
/** The preview's appearance: light, or dark with `?appearance=dark`. */
const appearance = new URLSearchParams(window.location.search).get("appearance") === "dark" ? "dark" : "light";
const messages = shellMessages(locale);
document.documentElement.lang = locale;

/** The preview's platform: every region in place, and a played search (fixtures.ts). */
const platform: ShellPlatform = { ...inPlace, search: previewSearch(previewSnapshot(messages), messages.language) };

function Preview() {
  const [snapshot, setSnapshot] = useState(() => previewSnapshot(messages));
  const actions: ShellActions = {
    activateApp: (id) =>
      console.info("[shell preview] activate app intent", id),
    focusWindow: (id) =>
      console.info("[shell preview] focus surface intent", id),
    closeWindow: (id) =>
      setSnapshot((current) => ({
        ...current,
        windows: current.windows.filter((win) => win.id !== id),
      })),
    activateWorkspace: (id) =>
      setSnapshot((current) => ({ ...current, workspaceId: id })),
    moveWindowToWorkspace: (windowId, workspaceId) =>
      setSnapshot((current) =>
        current.workspaces.some((workspace) => workspace.id === workspaceId)
          ? {
              ...current,
              windows: current.windows.map((window) =>
                window.id === windowId ? { ...window, workspaceId } : window,
              ),
            }
          : current,
      ),
    addWorkspace: () =>
      setSnapshot((current) =>
        current.workspaces.length >= 6
          ? current
          : {
              ...current,
              workspaces: [
                ...current.workspaces,
                {
                  id: String(current.workspaces.length + 1),
                  name: messages.overview.desktop(current.workspaces.length + 1),
                },
              ],
            },
      ),
    menuCommand: (id) =>
      console.info("[shell preview] system command intent", id),
    openFiles: (id, uris) => console.info("[shell preview] open files intent", id, uris),
    openUrl: (url) => console.info("[shell preview] open URL intent", url),
  };
  // A played machine (docs/adr/0025-hardware-keys-and-control-center.mdx), so the Control
  // Center can be tried in the browser: its changes are the fixture's alone.
  const [machine, setMachine] = useState<ShellControls>({
    levels: {
      output: { level: 0.5, muted: false, available: true },
      display: { level: 0.75, available: true },
    },
    power: { battery: "discharging", percent: 64, onAC: false, minutes: 200 },
    network: { wifi: "connected", signal: 73, ssid: "Oxy", wired: false },
  });
  // The played networks (docs/adr/0038-networks.mdx): a known or open network is joined in a
  // moment; a protected one asks its password in the sheet (one starting "mal" is wrong); one
  // OxyOS cannot join says so. As the session does, never the shell.
  const [networks, setNetworks] = useState<NetworkState>(previewNetworks);
  // The played sound (docs/adr/0056-sound-devices.mdx): an output chosen is played to at once.
  const [sound, setSound] = useState<SoundState>(previewSound);
  const [sheet, setSheet] = useState<{ id: number; text: SheetText } | null>(null);
  const sheets = useRef(0);
  const ask = (text: SheetText) => {
    sheets.current += 1;
    setSheet({ id: sheets.current, text });
  };
  const joined = (ssid: string) => {
    setNetworks((current) => {
      const networks = current.wifi.networks.map((network) => ({
        ...network,
        current: network.ssid === ssid,
        known: network.known || network.ssid === ssid,
      }));
      return {
        ...current,
        joining: null,
        wifi: {
          ...current.wifi,
          state: "connected",
          current: networks.find((network) => network.current) ?? null,
          networks,
        },
      };
    });
    const signal = networks.wifi.networks.find((network) => network.ssid === ssid)?.signal ?? 0;
    setMachine((current) => ({ ...current, network: { wifi: "connected", signal, ssid, wired: false } }));
  };
  const joinNow = (ssid: string) => {
    setNetworks((current) => ({ ...current, joining: ssid }));
    window.setTimeout(() => joined(ssid), 1200);
  };
  const join = (ssid: string) => {
    const network = networks.wifi.networks.find((candidate) => candidate.ssid === ssid);
    if (!network) return;
    if (!network.supported) ask({ kind: "wifi-unsupported", ssid, security: network.security });
    else if (network.known || !isProtected(network)) joinNow(ssid);
    else {
      setNetworks((current) => ({ ...current, joining: ssid }));
      ask({ kind: "wifi-password", ssid, security: network.security, incorrect: false });
    }
  };
  // The played Bluetooth (docs/adr/0055-bluetooth.mdx): a paired device connects or disconnects in
  // a moment; the mouse is out of reach, and its connection fails in the session's sheet.
  const [bluetoothDevices, setBluetoothDevices] = useState<BluetoothState>(previewBluetooth);
  const changeDevice = (id: string, change: Partial<BluetoothState["devices"][number]>) =>
    setBluetoothDevices((current) => ({
      ...current,
      devices: current.devices.map((device) => (device.id === id ? { ...device, ...change } : device)),
    }));
  const connectDevice = (id: string) => {
    const device = bluetoothDevices.devices.find((candidate) => candidate.id === id);
    if (!device) return;
    changeDevice(id, { busy: "connecting" });
    window.setTimeout(() => {
      if (device.kind === "mouse") {
        changeDevice(id, { busy: null });
        ask({ kind: "bluetooth-failed", name: device.name, device: device.kind, reason: "timeout" });
      } else changeDevice(id, { busy: null, connected: true });
    }, 1200);
  };
  const disconnectDevice = (id: string) => {
    changeDevice(id, { busy: "disconnecting" });
    window.setTimeout(() => changeDevice(id, { busy: null, connected: false }), 600);
  };
  // Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx),
  // played: the session's question is shown as a session would show it, and nothing ever ends,
  // sleeps or asks an application anything here.
  const powerActions: ShellPowerActions = {
    sleep: () => {},
    endSession: (action, confirm) => {
      if (confirm) ask({ kind: "end-session", action, seconds: 60, reopenWindows: true });
    },
  };
  const shownSheet = sheet && {
    id: sheet.id,
    text: sheet.text,
    answer: (_accepted: boolean) => {
      setSheet(null);
      setNetworks((current) => ({ ...current, joining: null }));
    },
    answerPassword: (password: string) => {
      setSheet(null);
      if (sheet.text.kind !== "wifi-password") return;
      const { ssid, security } = sheet.text;
      if (password.startsWith("mal")) ask({ kind: "wifi-password", ssid, security, incorrect: true });
      else joinNow(ssid);
    },
  };
  const controls: ShellControls = {
    ...machine,
    setLevel: (control, level) =>
      setMachine((current) => ({
        ...current,
        levels: {
          ...current.levels,
          [control]: { ...current.levels[control], level, muted: false, available: true },
        },
      })),
    setMuted: (control, muted) =>
      setMachine((current) => ({
        ...current,
        levels: {
          ...current.levels,
          [control]: { ...current.levels[control], level: current.levels[control]?.level ?? 0, muted, available: true },
        },
      })),
    setWifi: (enabled) => {
      setMachine((current) =>
        current.network
          ? {
              ...current,
              network: { ...current.network, wifi: enabled ? "connected" : "off" },
            }
          : current,
      );
      setNetworks((current) => ({
        ...current,
        wifi: { ...current.wifi, enabled, state: enabled ? "connected" : "off" },
      }));
    },
    setDark: (dark) => console.info("[shell preview] appearance intent", dark ? "dark" : "light"),
    networks,
    scanNetworks: () => console.info("[shell preview] scan intent"),
    joinNetwork: (ssid) => join(ssid),
    openWifiSettings: () => console.info("[shell preview] Wi-Fi settings intent"),
    sound,
    selectOutput: (id) =>
      setSound((current) => ({
        ...current,
        output: {
          ...current.output,
          devices: current.output.devices.map((device) => ({ ...device, selected: device.id === id })),
        },
      })),
    openSoundSettings: () => console.info("[shell preview] Sound settings intent"),
    bluetooth: bluetoothSummary(bluetoothDevices),
    bluetoothDevices,
    setBluetooth: (on) =>
      setBluetoothDevices((current) => ({
        ...current,
        adapter: current.adapter && { ...current.adapter, powered: on },
        // Off, every device is disconnected.
        devices: on ? current.devices : current.devices.map((device) => ({ ...device, connected: false, busy: null })),
      })),
    connectBluetooth: connectDevice,
    disconnectBluetooth: disconnectDevice,
    openBluetoothSettings: () => console.info("[shell preview] Bluetooth settings intent", "oxyos-settings://bluetooth"),
  };
  const [now, setNow] = useState(() => new Date());
  const [command, setCommand] = useState<ShellCommand>();
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000 * 30);
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape")
        setCommand((previous) => ({
          id: (previous?.id ?? 0) + 1,
          action: "dismiss",
        }));
    };
    // Ctrl+Space stands for Super+Space (Spotlight), Ctrl+Shift+Space for the launcher: system
    // shortcuts, taken before the focused field as the compositor takes them.
    const handleShortcut = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.code !== "Space") return;
      event.preventDefault();
      event.stopPropagation();
      setCommand((previous) => ({
        id: (previous?.id ?? 0) + 1,
        action: event.shiftKey ? "launcher" : "spotlight",
      }));
    };
    window.addEventListener("keydown", handleKey);
    window.addEventListener("keydown", handleShortcut, true);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("keydown", handleKey);
      window.removeEventListener("keydown", handleShortcut, true);
    };
  }, []);
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <BloomProvider
          mode={appearance}
          colorPreset="graphite"
          haptics={false}
          locale={locale}
        >
          <Shell
            snapshot={snapshot}
            actions={actions}
            wallpaper={previewWallpaper}
            now={now}
            command={command}
            controls={controls}
            sheet={shownSheet ?? undefined}
            platform={platform}
            powerActions={powerActions}
          />
        </BloomProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
/** Activity Monitor alone (`?app=activity-monitor`), over a played machine (activity-fixture.ts). */
function ActivityPreview() {
  const source = React.useMemo(previewActivity, []);
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <BloomProvider mode={appearance} colorPreset="graphite" haptics={false} locale={locale}>
          <ActivityMonitor source={source} locale={locale} />
        </BloomProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const app = new URLSearchParams(window.location.search).get("app");
createRoot(document.getElementById("root")!).render(app === "activity-monitor" ? <ActivityPreview /> : <Preview />);
