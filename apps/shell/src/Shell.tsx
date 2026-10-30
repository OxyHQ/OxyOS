import React, { useEffect, useRef, useState } from "react";
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
import { AppSwitcher } from "./components/AppSwitcher";
import {
  chosen,
  followSwitcher,
  moveSwitcher,
  openSwitcher,
  type Switcher,
} from "./switcher";
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
  commands,
  lock,
  platform = inPlace,
  dock = true,
}: {
  snapshot: ShellSnapshot;
  actions: ShellActions;
  wallpaper: ImageSourcePropType;
  now: Date;
  command?: ShellCommand;
  /** The latest commands, each with a rising `id`: every one not handled yet is, in order (the
      system shortcuts, which may come several at once, docs/adr/0024-system-shortcuts.mdx). */
  commands?: readonly ShellCommand[];
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
  const [searchRequest, setSearchRequest] = useState(0);
  // The application switcher (docs/adr/0024-system-shortcuts.mdx), open while Super is held.
  const [switcher, setSwitcher] = useState<Switcher | null>(null);
  // Commands act on what is shown now, whichever render they arrive in.
  const latest = useRef({ snapshot, actions, switcher, view });
  latest.current = { snapshot, actions, switcher, view };
  const dismiss = () => setView("desktop");
  const toggle = (next: typeof view) =>
    setView((current) => (current === next ? "desktop" : next));
  const activateApp = (id: string) => {
    actions.activateApp(id);
    dismiss();
  };
  // Each command once, in order: several may come in one render (Super+Tab released at once
  // opens the switcher and ends it with its choice, as a Mac's quick ⌘Tab switches back).
  const done = useRef(0);
  useEffect(() => {
    const pending = [...(commands ?? []), ...(command ? [command] : [])]
      .filter((next) => next.id > done.current)
      .sort((a, b) => a.id - b.id);
    if (pending.length === 0) return;
    done.current = pending[pending.length - 1].id;
    const now = latest.current;
    let current = now.switcher;
    let view = now.view;
    for (const next of pending) {
      switch (next.action) {
        case "dismiss":
          setEscapeRequest((value) => value + 1);
          break;
        case "launcher":
        case "overview":
          view = next.action;
          break;
        // Super+Space: the launcher, its search taking what is typed; again, it closes.
        case "search":
          if (view === "launcher") {
            view = "desktop";
          } else {
            view = "launcher";
            setSearchRequest((value) => value + 1);
          }
          break;
        case "switcher-next":
        case "switcher-previous": {
          const backwards = next.action === "switcher-previous";
          current = current
            ? moveSwitcher(current, backwards ? -1 : 1)
            : openSwitcher(now.snapshot, backwards);
          break;
        }
        // Its choice is quit or hidden; it stays, and follows what still runs.
        case "switcher-quit":
          if (current) now.actions.quitApp?.(chosen(current));
          break;
        case "switcher-hide":
          if (current) now.actions.hideApp?.(chosen(current));
          break;
        case "switcher-done":
          if (current) {
            now.actions.activateApp(chosen(current));
            view = "desktop";
          }
          current = null;
          break;
        case "switcher-cancel":
          current = null;
          break;
      }
    }
    setSwitcher(current);
    setView(view);
  }, [command, commands]);
  // An application quit while the switcher is open leaves it.
  useEffect(() => {
    setSwitcher((current) => current && followSwitcher(current, snapshot));
  }, [snapshot]);
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
            searchRequest={searchRequest}
            applications={snapshot.applications}
            folders={snapshot.folders}
            wallpaper={wallpaper}
            launch={activateApp}
            dismiss={dismiss}
          />
        </Surface>
        {switcher && (
          <Surface
            kind="overlay"
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              right: 0,
              bottom: 0,
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <AppSwitcher
              switcher={switcher}
              applications={snapshot.applications}
              activate={(id) => {
                setSwitcher(null);
                activateApp(id);
              }}
            />
          </Surface>
        )}
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
