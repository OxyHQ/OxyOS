import React, { useEffect, useState } from "react";
import { StyleSheet, type ImageSourcePropType } from "react-native";
import { BloomScope } from "@oxy.so/bloom/appearance";
import { Box } from "./primitives";
import type { ShellActions, ShellCommand, ShellLock, ShellSnapshot } from "./model";
import { Wallpaper } from "./components/Wallpaper";
import { TopBar } from "./components/TopBar";
import { DOCK_HEIGHT, Dock } from "./components/Dock";
import { Overview } from "./components/Overview";
import { Launchpad } from "./components/Launchpad";
import { LockScreen } from "./components/LockScreen";
import { inPlace, type ShellPlatform } from "./platform";
export type { ShellCommand } from "./model";
export type {
  ShellPlatform,
  ShellSurfaceProps,
  WindowViewProps,
} from "./platform";

/** OxyOS shell presentation. Native application surfaces belong to the compositor beneath it. */
export function Shell({
  snapshot,
  actions,
  wallpaper,
  now,
  command,
  lock,
  platform = inPlace,
  dock = true,
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  wallpaper: ImageSourcePropType;
  now: Date;
  command?: ShellCommand;
  /** The session's lock, where the platform has one: the lock screen, and the menu's item. */
  lock?: ShellLock;
  /** Where the regions are drawn: separate surfaces on OxyOS, in place elsewhere. */
  platform?: ShellPlatform;
  /**
   * Whether this display has the dock: the primary has it, as a Mac's main display has the Dock;
   * every display has the top bar (docs/adr/0018-several-displays.mdx).
   */
  dock?: boolean;
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
    else if (id === "session.lock") lock?.lockSession();
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
            canLock={!!lock}
            onCommand={menuCommand}
          />
        </Surface>
        <Box
          className="min-h-0 flex-1"
          pointerEvents={view === "desktop" ? "box-none" : "auto"}
        >
          {view === "overview" && (
            <Surface kind="overlay" style={StyleSheet.absoluteFill}>
              <Overview
                snapshot={snapshot}
                actions={actions}
                dismiss={dismiss}
                escapeRequest={escapeRequest}
                WindowView={platform.WindowView}
              />
            </Surface>
          )}
        </Box>
        {/* The launcher covers the whole screen, the top bar included, but the dock: as Launchpad
            covers the menu bar and leaves the Dock, whose button closes it again. It is built
            once, when the shell starts, and kept while hidden, as macOS keeps Launchpad: opening
            it shows what is built. Hidden, it has no surface and takes no input. */}
        <Surface
          kind="overlay"
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            right: 0,
            bottom: dock ? DOCK_HEIGHT : 0,
            display: view === "launcher" ? "flex" : "none",
          }}
        >
          <Launchpad
            open={view === "launcher"}
            escapeRequest={escapeRequest}
            applications={snapshot.applications}
            folders={snapshot.folders}
            wallpaper={wallpaper}
            launch={activateApp}
            dismiss={dismiss}
          />
        </Surface>
        {dock && (
          <Surface kind="panel" edge="bottom" exclusive>
            <Dock
              snapshot={snapshot}
              view={view}
              activateApp={activateApp}
              toggleLauncher={() => toggle("launcher")}
              toggleOverview={() => toggle("overview")}
            />
          </Surface>
        )}
        {/* The lock screen: a surface of its own, all the compositor shows while locked. The
            desktop stays as it was beneath, as it is when the session unlocks. */}
        {lock?.locked && (
          <Surface kind="lock" style={StyleSheet.absoluteFill}>
            <LockScreen
              lock={lock}
              wallpaper={wallpaper}
              now={now}
              Masked={platform.Masked}
            />
          </Surface>
        )}
      </Box>
    </BloomScope>
  );
}
