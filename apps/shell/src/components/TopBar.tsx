import React, { useEffect, useRef, useState } from "react";
import { AccessibilityInfo, View } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
} from "@oxy.so/bloom/dropdown-menu";
import { RiSearchLine } from "@oxy.so/bloom/icons/RiSearchLine";

import Svg, { Rect, Circle, Path } from "react-native-svg";
import { Box } from "../primitives";
import { OxyLogo } from "./OxyLogo";
import { batteryLabel, wifiBars, wifiLabel, wifiOn } from "../controls";
import type { ShellBattery, ShellControls, ShellMenu, ShellMenuItem, ShellNetwork } from "../model";
import type { ShellKeyboard } from "../platform";
import { barKey } from "../menu-bar-keys";

/** A menu's entries as Bloom's rows: commands (with their check and chord), submenus and
    separators. A choice calls `onCommand` with the entry's id; the menu closes. */
function MenuEntries({
  items,
  onCommand,
}: {
  items: readonly ShellMenuItem[];
  onCommand: (id: string) => void;
}) {
  return (
    <>
      {items.map((item) => {
        if (item.kind === "separator") {
          return (
            <React.Fragment key={item.id}>
              <DropdownMenuSeparator />
            </React.Fragment>
          );
        }
        if (item.kind === "submenu") {
          return (
            <DropdownMenuSub key={item.id}>
              <DropdownMenuSubTrigger disabled={item.enabled === false}>
                {item.label}
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <MenuEntries items={item.items} onCommand={onCommand} />
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          );
        }
        const chord = item.shortcut ? (
          <DropdownMenuShortcut>{item.shortcut}</DropdownMenuShortcut>
        ) : undefined;
        if (item.checked !== undefined) {
          return (
            <DropdownMenuCheckboxItem
              key={item.id}
              checked={item.checked}
              disabled={item.enabled === false}
              trailing={chord}
              onCheckedChange={() => onCommand(item.id)}
            >
              {item.label}
            </DropdownMenuCheckboxItem>
          );
        }
        return (
          <DropdownMenuItem
            key={item.id}
            disabled={item.enabled === false}
            trailing={chord}
            onPress={() => onCommand(item.id)}
          >
            {item.label}
          </DropdownMenuItem>
        );
      })}
    </>
  );
}

function ControlsIcon({
  width = 15,
  height = 15,
  fill = "#fff",
}: {
  width?: number;
  height?: number;
  fill?: string;
}) {
  return (
    <Svg width={width} height={height} viewBox="0 0 16 16">
      <Rect
        x="1"
        y="2"
        width="14"
        height="4"
        rx="2"
        stroke={fill}
        strokeWidth="1.2"
        fill="none"
      />
      <Circle cx="5" cy="4" r="1.5" fill={fill} />
      <Rect
        x="1"
        y="10"
        width="14"
        height="4"
        rx="2"
        stroke={fill}
        strokeWidth="1.2"
        fill="none"
      />
      <Circle cx="11" cy="12" r="1.5" fill={fill} />
    </Svg>
  );
}

/** The battery as the session says it is: its charge filled in, a bolt while it charges. */
function BatteryIcon({ power }: { power: ShellBattery }) {
  const fill = Math.max(1, Math.round((17 * Math.min(100, power.percent)) / 100));
  const low = power.battery === "discharging" && power.percent <= 10;
  return (
    <Svg width={24} height={12} viewBox="0 0 26 13">
      <Rect x="1" y="1" width="21" height="11" rx="3" stroke="#ffffffcc" fill="none" />
      <Rect x="3" y="3" width={fill} height="7" rx="1.5" fill={low ? "#ff5a4f" : "#fff"} />
      <Rect x="23.5" y="4" width="1.5" height="5" rx=".7" fill="#ffffffb3" />
      {(power.battery === "charging" || power.battery === "full") && (
        <Path d="M12.6 1.6 8.4 7.1h3l-1 4.3 4.3-5.6h-3.1l1-4.2Z" fill="#26313d" stroke="#fff" strokeWidth=".6" />
      )}
    </Svg>
  );
}

/** Wi-Fi as the session says it is: its signal in arcs; off or not connected, faint. */
function WifiIcon({ network }: { network: ShellNetwork }) {
  const connected = network.wifi === "connected";
  const bars = connected ? wifiBars(network.signal) : 0;
  const arc = (on: boolean) => (on ? "#fff" : "#ffffff59");
  return (
    <Svg width={14} height={12} viewBox="0 0 20 16">
      <Path d="M1 5.6a13 13 0 0 1 18 0" stroke={arc(bars >= 3)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M4.2 8.9a8.4 8.4 0 0 1 11.6 0" stroke={arc(bars >= 2)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M7.4 12.1a3.9 3.9 0 0 1 5.2 0" stroke={arc(bars >= 1)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Circle cx="10" cy="14.6" r="1.3" fill={arc(connected || wifiOn(network))} />
    </Svg>
  );
}

const menuStyle = {
  height: 28,
  paddingLeft: 8,
  paddingRight: 8,
  backgroundColor: "transparent",
  borderWidth: 0,
  borderRadius: 4,
};
const menuText = { color: "#fff", fontSize: 12, lineHeight: 16 };
/** A menu's title with the keyboard on it, as a Mac's bar highlights it. */
const keyboardHighlight = "#ffffff38";
export function TopBar({
  menus,
  now,
  overview,
  launcher,
  canLock = false,
  onCommand,
  controls,
  controlCenter,
  clock,
  keyboardRequest = 0,
  keyboardLost = 0,
  keyboard,
  onKeyboard,
  returnKeyboard,
}: {
  menus: readonly ShellMenu[];
  now: Date;
  overview: () => void;
  launcher: () => void;
  /** Whether the session can be locked from here (the platform has a lock). */
  canLock?: boolean;
  onCommand: (id: string) => void;
  /** The machine as the session says it is (docs/adr/0025-hardware-keys-and-control-center.mdx):
      the battery and Wi-Fi are shown from it, and only what it said. */
  controls?: ShellControls;
  /** Opens the Control Center, or closes it. */
  controlCenter: () => void;
  /** The clock opens and closes the Notification Center, as a Mac's (docs/adr/0026-notifications.mdx). */
  clock?: () => void;
  /** Rises at each Super+F2 (docs/adr/0024-system-shortcuts.mdx): the keyboard moves to the
      bar's first menu, as a Mac's Control-F2. */
  keyboardRequest?: number;
  /** Rises when the keyboard left the bar and its menus (a press in a window): the bar lets go. */
  keyboardLost?: number;
  /** The platform's keys before anything else, which drive the bar while it has the keyboard. */
  keyboard?: ShellKeyboard;
  /** Whether the bar wants the keyboard: its surface asks the platform for it. */
  onKeyboard?: (wanted: boolean) => void;
  /** Gives the keyboard back to the window that had it (the active one), as the bar is left. */
  returnKeyboard?: () => void;
}) {
  const battery = batteryLabel(controls?.power);
  const wifi = wifiLabel(controls?.network);
  // The bar from the keyboard: the menu whose title has the keyboard (0 the Oxy menu, then the
  // active window's), and the one open; null when the keyboard is elsewhere.
  const count = menus.length + 1;
  const [focused, setFocused] = useState<number | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const titles = useRef<(React.ComponentRef<typeof View> | null)[]>([]);
  const state = useRef({ focused, open, count });
  state.current = { focused, open, count };
  const leave = (giveBack: boolean) => {
    setFocused(null);
    setOpen(null);
    onKeyboard?.(false);
    if (giveBack) returnKeyboard?.();
  };
  const first = useRef(keyboardRequest);
  useEffect(() => {
    if (keyboardRequest === first.current) return;
    first.current = keyboardRequest;
    setOpen(null);
    setFocused(0);
    onKeyboard?.(true);
  }, [keyboardRequest]);
  const lost = useRef(keyboardLost);
  useEffect(() => {
    if (keyboardLost === lost.current) return;
    lost.current = keyboardLost;
    if (state.current.focused !== null) leave(false);
  }, [keyboardLost]);
  // The assistive technology's focus follows the keyboard's title, as VoiceOver's does.
  useEffect(() => {
    if (focused === null || open !== null) return;
    const view = titles.current[focused];
    if (view) AccessibilityInfo.sendAccessibilityEvent(view, "focus");
  }, [focused, open]);
  // The menus the bar shows can change under the keyboard (another window active): it stays on
  // the bar, within it.
  useEffect(() => {
    if (focused !== null && focused >= count) setFocused(count - 1);
  }, [count, focused]);
  useEffect(() => {
    if (focused === null || !keyboard) return;
    return keyboard.subscribe((event) => {
      const now = state.current;
      if (now.focused === null) return false;
      const action = barKey(event, now.open !== null);
      switch (action) {
        case "previous":
        case "next": {
          const step = action === "next" ? 1 : -1;
          const to = (now.focused + step + now.count) % now.count;
          setFocused(to);
          if (now.open !== null) setOpen(to);
          return true;
        }
        case "open":
          setOpen(now.focused);
          return true;
        case "leave":
          leave(true);
          return true;
        default:
          return false;
      }
    });
  }, [focused === null, keyboard]);
  // A row chosen from the keyboard leaves the bar, the keyboard back with the window first, so
  // the command acts where the person was (or moves it on, as the launcher's search does).
  const choose = (run: () => void) => () => {
    if (state.current.focused !== null) leave(true);
    run();
  };
  const command = (id: string) => choose(() => onCommand(id))();
  const opened = (index: number) => ({
    open: open === index,
    onOpenChange: (next: boolean) =>
      setOpen((current) => (next ? index : current === index ? null : current)),
  });
  const title = (index: number) => (view: React.ComponentRef<typeof View> | null) => {
    titles.current[index] = view;
  };
  const highlight = (index: number) =>
    focused === index ? { backgroundColor: keyboardHighlight } : undefined;
  return (
    <Box
      testID="shell-topbar"
      className="h-7 shrink-0 flex-row items-center justify-between px-3"
      style={{ zIndex: 30 }}
    >
      <Box className="flex-row items-center gap-0">
        <DropdownMenu {...opened(0)}>
          <View ref={title(0)} collapsable={false} style={[{ borderRadius: 4 }, highlight(0)]}>
            <DropdownMenuTrigger asChild>
              <Button
                iconOnly
                icon={OxyLogo}
                accessibilityLabel="Menú de Oxy"
                appearance="plain"
                className="h-7 w-8 border-0 bg-transparent p-0"
                iconSize={17}
                colors={{ background: "transparent", foreground: "#fff" }}
              />
            </DropdownMenuTrigger>
          </View>
          <DropdownMenuContent
            align="start"
            label="Oxy"
            size="sm"
            minWidth={220}
            className="rounded-lg"
            style={{ borderRadius: 8 }}
          >
            <DropdownMenuItem onPress={() => command("system.settings")}>
              Ajustes del sistema
            </DropdownMenuItem>
            <DropdownMenuItem onPress={choose(launcher)}>Aplicaciones</DropdownMenuItem>
            <DropdownMenuItem onPress={choose(overview)}>
              Ver escritorios
            </DropdownMenuItem>
            {canLock && (
              <DropdownMenuItem onPress={() => command("session.lock")}>
                Bloquear pantalla
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        {menus.map((menu, index) => (
          <DropdownMenu key={`${index}:${menu.label}`} {...opened(index + 1)}>
            <View ref={title(index + 1)} collapsable={false} style={[{ borderRadius: 4 }, highlight(index + 1)]}>
              <DropdownMenuTrigger asChild>
                <Button
                  appearance="plain"
                  className="h-7 rounded border-0 bg-transparent px-2"
                  style={menuStyle}
                  textStyle={{
                    ...menuText,
                    fontWeight: index === 0 ? "600" : "400",
                  }}
                >
                  <Text
                    style={{
                      ...menuText,
                      fontWeight: index === 0 ? "600" : "400",
                    }}
                  >
                    {menu.label}
                  </Text>
                </Button>
              </DropdownMenuTrigger>
            </View>
            <DropdownMenuContent
              align="start"
              label={menu.label}
              size="sm"
              minWidth={220}
              className="rounded-lg"
              style={{ borderRadius: 8 }}
            >
              <MenuEntries items={menu.items} onCommand={command} />
            </DropdownMenuContent>
          </DropdownMenu>
        ))}
      </Box>
      <Box className="flex-row items-center gap-2.5">
        <Box className="flex-row items-center gap-4">
          {battery !== undefined && controls?.power && (
            <Box testID="shell-battery" accessible accessibilityLabel={`Batería: ${battery}`}>
              <BatteryIcon power={controls.power} />
            </Box>
          )}
          {wifi !== undefined && controls?.network && (
            <Box testID="shell-wifi" accessible accessibilityLabel={`Wi-Fi: ${wifi}`}>
              <WifiIcon network={controls.network} />
            </Box>
          )}
        </Box>
        <Button
          iconOnly
          icon={RiSearchLine}
          accessibilityLabel="Buscar aplicaciones"
          appearance="plain"
          style={{ ...menuStyle, width: 22, paddingHorizontal: 0 }}
          iconSize={13}
          colors={{ background: "transparent", foreground: "#fff" }}
          onPress={launcher}
        />
        <Button
          iconOnly
          icon={ControlsIcon}
          iconSize={15}
          testID="shell-control-center"
          accessibilityLabel="Centro de control"
          appearance="plain"
          className="h-7 w-[22px] border-0 bg-transparent p-0"
          colors={{ background: "transparent", foreground: "#fff" }}
          onPress={controlCenter}
        />
        <Button
          testID="shell-clock"
          appearance="plain"
          accessibilityLabel="Centro de notificaciones"
          // Where the clock's text always was (its old `ml-1`), now pressable.
          style={{ ...menuStyle, paddingLeft: 4, paddingRight: 0 }}
          onPress={clock}
        >
          <Text className="text-[12px] leading-4 font-normal text-white">
            {now.toLocaleDateString("es", {
              weekday: "short",
              day: "numeric",
              month: "short",
            })}{" "}
            {now.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" })}
          </Text>
        </Button>
      </Box>
    </Box>
  );
}
