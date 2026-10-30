import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { BloomProvider } from "@oxy.so/bloom/provider";
import { Shell, type ShellCommand } from "../../../apps/shell/src/Shell";
import "./styles.css";
import { previewSnapshot, previewWallpaper } from "./fixtures";
import type {
  ShellActions,
  ShellControls,
} from "../../../apps/shell/src/model";

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
    setWifi: (enabled) =>
      setMachine((current) =>
        current.network
          ? {
              ...current,
              network: { ...current.network, wifi: enabled ? "connected" : "off" },
            }
          : current,
      ),
    setDark: (dark) => console.info("[shell preview] appearance intent", dark ? "dark" : "light"),
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
          />
        </BloomProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
createRoot(document.getElementById("root")!).render(<Preview />);
