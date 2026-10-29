import React, { useEffect, useState } from "react";
import { StyleSheet, type ImageSourcePropType } from "react-native";
import { BloomScope } from "@oxy.so/bloom/appearance";
import { Box } from "./primitives";
import type { ShellActions, ShellCommand, ShellSnapshot } from "./model";
import { Wallpaper } from "./components/Wallpaper";
import { TopBar } from "./components/TopBar";
import { Dock } from "./components/Dock";
import { Overview } from "./components/Overview";
import { Launchpad } from "./components/Launchpad";
import { inPlace, type ShellPlatform } from "./platform";
export type { ShellCommand } from "./model";
export type { ShellPlatform, ShellSurfaceProps } from "./platform";

/** OxyOS shell presentation. Native application surfaces belong to the compositor beneath it. */
export function Shell({
  snapshot,
  actions,
  wallpaper,
  now,
  command,
  platform = inPlace,
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  wallpaper: ImageSourcePropType;
  now: Date;
  command?: ShellCommand;
  /** Where the regions are drawn: separate surfaces on OxyOS, in place elsewhere. */
  platform?: ShellPlatform;
}) {
  const { Surface } = platform;
  const [view, setView] = useState<"desktop" | "launcher" | "overview">(
    "desktop",
  );
  const [escapeRequest, setEscapeRequest] = useState(0);
  useEffect(() => {
    if (!command) return;
    if (command.action === "dismiss") setEscapeRequest((value) => value + 1);
    else setView(command.action);
  }, [command]);
  const dismiss = () => setView("desktop");
  const toggle = (next: typeof view) =>
    setView((current) => (current === next ? "desktop" : next));
  const activateApp = (id: string) => {
    actions.activateApp(id);
    dismiss();
  };
  const menuCommand = (id: string) => {
    if (id === "view.launcher") setView("launcher");
    else if (id === "view.overview") setView("overview");
    else actions.menuCommand(id);
  };
  return (
    <BloomScope size="md">
      <Box testID="oxy-shell" className="flex-1 overflow-hidden bg-[#294ab4]">
        <Box className="absolute inset-0">
          <Wallpaper source={wallpaper} />
        </Box>
        <Surface kind="panel" edge="top" exclusive>
          <TopBar
            menus={snapshot.menus}
            now={now}
            overview={() => toggle("overview")}
            launcher={() => toggle("launcher")}
            onCommand={menuCommand}
          />
        </Surface>
        <Box
          className="min-h-0 flex-1"
          pointerEvents={view === "desktop" ? "box-none" : "auto"}
        >
          {view !== "desktop" && (
            <Surface kind="overlay" style={StyleSheet.absoluteFill}>
              {view === "overview" && (
                <Overview
                  snapshot={snapshot}
                  actions={actions}
                  dismiss={dismiss}
                  escapeRequest={escapeRequest}
                />
              )}
              {view === "launcher" && (
                <Launchpad
                  escapeRequest={escapeRequest}
                  applications={snapshot.applications}
                  folders={snapshot.folders}
                  launch={activateApp}
                  dismiss={dismiss}
                />
              )}
            </Surface>
          )}
        </Box>
        <Surface kind="panel" edge="bottom" exclusive>
          <Dock
            snapshot={snapshot}
            view={view}
            activateApp={activateApp}
            toggleLauncher={() => toggle("launcher")}
            toggleOverview={() => toggle("overview")}
          />
        </Surface>
      </Box>
    </BloomScope>
  );
}
