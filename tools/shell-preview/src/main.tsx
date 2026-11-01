import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { BloomProvider } from "@oxy.so/bloom/provider";
import { Shell, type ShellCommand } from "../../../apps/shell/src/Shell";
import "./styles.css";
import { previewNetworks, previewSnapshot, previewWallpaper } from "./fixtures";
import type {
  ShellActions,
  ShellControls,
} from "../../../apps/shell/src/model";
import { isProtected, type NetworkState } from "../../../apps/shell/src/network";
import type { SheetText } from "../../../apps/shell/src/sheet";

function Preview() {
  const [snapshot, setSnapshot] = useState(previewSnapshot);
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
                  name: `Escritorio ${current.workspaces.length + 1}`,
                },
              ],
            },
      ),
    menuCommand: (id) =>
      console.info("[shell preview] system command intent", id),
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
    bluetooth: { state: "on", connected: 1 },
  });
  // The played networks (docs/adr/0038-networks.mdx): a known or open network is joined in a
  // moment; a protected one asks its password in the sheet (one starting "mal" is wrong); one
  // OxyOS cannot join says so. As the session does, never the shell.
  const [networks, setNetworks] = useState<NetworkState>(previewNetworks);
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
  const shownSheet = sheet && {
    id: sheet.id,
    text: sheet.text,
    language: "es" as const,
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
      if (event.ctrlKey && event.code === "Space") {
        event.preventDefault();
        setCommand((previous) => ({
          id: (previous?.id ?? 0) + 1,
          action: "launcher",
        }));
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("keydown", handleKey);
    };
  }, []);
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <BloomProvider
          mode="light"
          colorPreset="graphite"
          haptics={false}
          locale="es"
        >
          <Shell
            snapshot={snapshot}
            actions={actions}
            wallpaper={previewWallpaper}
            now={now}
            command={command}
            controls={controls}
            sheet={shownSheet ?? undefined}
          />
        </BloomProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
createRoot(document.getElementById("root")!).render(<Preview />);
