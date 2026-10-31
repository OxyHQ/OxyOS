import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import {
  Menubar,
  MenubarMenu,
  MenubarTrigger,
  MenubarContent,
  MenubarItem,
  MenubarCheckboxItem,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubTrigger,
  MenubarSubContent,
} from "@oxy.so/bloom/menubar";
import { RiSearchLine } from "@oxy.so/bloom/icons/RiSearchLine";

import Svg, { Rect, Circle, Path } from "react-native-svg";
import { Box } from "../primitives";
import { OxyLogo } from "./OxyLogo";
import { batteryLabel, wifiBars, wifiLabel, wifiOn } from "../controls";
import type { ShellBattery, ShellMenu, ShellMenuItem, ShellNetwork } from "../model";

/** A menu's entries as Bloom's menu bar rows: commands (with their check and chord), submenus
    and separators. A choice calls `onCommand` with the entry's id; the menu closes. */
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
              <MenubarSeparator />
            </React.Fragment>
          );
        }
        if (item.kind === "submenu") {
          return (
            <MenubarSub key={item.id}>
              <MenubarSubTrigger disabled={item.enabled === false}>{item.label}</MenubarSubTrigger>
              <MenubarSubContent label={item.label}>
                <MenuEntries items={item.items} onCommand={onCommand} />
              </MenubarSubContent>
            </MenubarSub>
          );
        }
        // The chord as the session shows it (Ctrl, not the application's `Mod`), as the row's
        // trailing hint. Never the row's `shortcut`: the application runs its own chords, and
        // the shell would run them a second time.
        const chord = item.shortcut ? <MenubarShortcut>{item.shortcut}</MenubarShortcut> : undefined;
        if (item.checked !== undefined) {
          return (
            <MenubarCheckboxItem
              key={item.id}
              checked={item.checked}
              disabled={item.enabled === false}
              trailing={chord}
              onCheckedChange={() => onCommand(item.id)}
            >
              {item.label}
            </MenubarCheckboxItem>
          );
        }
        return (
          <MenubarItem
            key={item.id}
            disabled={item.enabled === false}
            trailing={chord}
            onPress={() => onCommand(item.id)}
          >
            {item.label}
          </MenubarItem>
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
/** The bar is the panel's own row over the wallpaper: none of Bloom's in-window bar chrome. */
const barStyle = {
  height: 28,
  paddingTop: 0,
  paddingBottom: 0,
  paddingLeft: 0,
  paddingRight: 0,
  gap: 0,
  borderWidth: 0,
  borderRadius: 0,
  backgroundColor: "transparent",
  boxShadow: "0 0 0 0 transparent",
};
/** The Oxy menu's value in the bar; the window's menus are theirs by place and name. */
const OXY = "oxy";
const menuValue = (index: number, menu: ShellMenu) => `${index}:${menu.label}`;
/** A menu hangs just under the bar, its rows' text under its title's (Bloom's own `alignOffset`). */
const MENU_SIDE_OFFSET = 4;

/** The bar's status items: the battery and Wi-Fi as the session says they are, and only what it
    said; each renders again only when what it shows changes. */
const BatteryItem = memo(function BatteryItem({ power }: { power: ShellBattery }) {
  const battery = batteryLabel(power);
  if (battery === undefined) return null;
  return (
    <Box testID="shell-battery" accessible accessibilityLabel={`Batería: ${battery}`}>
      <BatteryIcon power={power} />
    </Box>
  );
});

const WifiItem = memo(function WifiItem({ network }: { network: ShellNetwork }) {
  const wifi = wifiLabel(network);
  if (wifi === undefined) return null;
  return (
    <Box testID="shell-wifi" accessible accessibilityLabel={`Wi-Fi: ${wifi}`}>
      <WifiIcon network={network} />
    </Box>
  );
});

const TOP_BAR_STYLE = { zIndex: 30 };
const TRANSPARENT_WHITE = { background: "transparent", foreground: "#fff" };
const searchStyle = { ...menuStyle, width: 22, paddingHorizontal: 0 };
const clockStyle = { ...menuStyle, paddingLeft: 4, paddingRight: 0 };
const DATE_FORMAT = { weekday: "short", day: "numeric", month: "short" } as const;
const TIME_FORMAT = { hour: "2-digit", minute: "2-digit" } as const;

/** The search and the Control Center's buttons. */
const BarButtons = memo(function BarButtons({
  launcher,
  controlCenter,
}: {
  launcher: () => void;
  controlCenter: () => void;
}) {
  return (
    <>
      <Button
        iconOnly
        icon={RiSearchLine}
        accessibilityLabel="Buscar aplicaciones"
        appearance="plain"
        style={searchStyle}
        iconSize={13}
        colors={TRANSPARENT_WHITE}
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
        colors={TRANSPARENT_WHITE}
        onPress={controlCenter}
      />
    </>
  );
});

/** The clock: the date and the time, which open the Notification Center. */
const Clock = memo(function Clock({ now, onPress }: { now: Date; onPress?: () => void }) {
  return (
    <Button
      testID="shell-clock"
      appearance="plain"
      accessibilityLabel="Centro de notificaciones"
      // Where the clock's text always was (its old `ml-1`), now pressable.
      style={clockStyle}
      onPress={onPress}
    >
      <Text className="text-[12px] leading-4 font-normal text-white">
        {now.toLocaleDateString("es", DATE_FORMAT)} {now.toLocaleTimeString("es", TIME_FORMAT)}
      </Text>
    </Button>
  );
});

/**
 * The menus of the bar: Oxy's, then the active window's (or the shell's). It renders again only
 * when they change, a menu opens or the keyboard moves among them; never for the clock or the
 * battery.
 */
const BarMenus = memo(function BarMenus({
  menus,
  overview,
  launcher,
  canLock,
  onCommand,
  keyboardRequest,
  keyboardLost,
  onKeyboard,
  returnKeyboard,
}: {
  menus: readonly ShellMenu[];
  overview: () => void;
  launcher: () => void;
  canLock: boolean;
  onCommand: (id: string) => void;
  keyboardRequest: number;
  keyboardLost: number;
  onKeyboard?: (wanted: boolean) => void;
  returnKeyboard?: () => void;
}) {
  // The bar is Bloom's Menubar: the menu open, and the menu whose title has the keyboard (the
  // bar driven from it, Bloom's `focusValue`), undefined when the keyboard is elsewhere. Bloom
  // takes the bar's keys while it has the keyboard (← →, ↓ Return Space, a letter, Escape) and
  // the open menu's (its rows, its submenus).
  const values = useMemo(() => [OXY, ...menus.map((menu, index) => menuValue(index, menu))], [menus]);
  const [open, setOpen] = useState<string | undefined>(undefined);
  const [focused, setFocused] = useState<string | undefined>(undefined);
  // Whether the bar holds the keyboard: leaving is said to the platform once, however many ways
  // it is left at once (a row chosen leaves before its command runs, and Bloom leaves again).
  const holding = useRef(false);
  const leave = useCallback(
    (giveBack: boolean) => {
      setFocused(undefined);
      setOpen(undefined);
      if (!holding.current) return;
      holding.current = false;
      onKeyboard?.(false);
      if (giveBack) returnKeyboard?.();
    },
    [onKeyboard, returnKeyboard],
  );
  const first = useRef(keyboardRequest);
  useEffect(() => {
    if (keyboardRequest === first.current) return;
    first.current = keyboardRequest;
    setOpen(undefined);
    setFocused(OXY);
    holding.current = true;
    onKeyboard?.(true);
  }, [keyboardRequest]);
  const lost = useRef(keyboardLost);
  useEffect(() => {
    if (keyboardLost === lost.current) return;
    lost.current = keyboardLost;
    if (holding.current) leave(false);
  }, [keyboardLost]);
  // The menus the bar shows can change under the keyboard (another window active): it stays on
  // the bar, on its last menu.
  const last = values[values.length - 1];
  useEffect(() => {
    if (focused !== undefined && !values.includes(focused)) setFocused(last);
  }, [focused, values]);
  // A row chosen from the keyboard leaves the bar, the keyboard back with the window first, so
  // the command acts where the person was (or moves it on, as the launcher's search does).
  const choose = (run: () => void) => () => {
    if (holding.current) leave(true);
    run();
  };
  const command = (id: string) => choose(() => onCommand(id))();
  const highlight = (value: string) =>
    focused === value ? { backgroundColor: keyboardHighlight } : undefined;
  return (
    <Menubar
      label="Barra de menús"
      value={open}
      onValueChange={setOpen}
      focusValue={focused}
      onFocusValueChange={(value) => (value === undefined ? leave(true) : setFocused(value))}
      style={barStyle}
    >
      <MenubarMenu value={OXY}>
        <MenubarTrigger asChild label="Menú de Oxy" style={[{ borderRadius: 4 }, highlight(OXY)]}>
          <Button
            iconOnly
            icon={OxyLogo}
            accessibilityLabel="Menú de Oxy"
            appearance="plain"
            className="h-7 w-8 border-0 bg-transparent p-0"
            iconSize={17}
            colors={TRANSPARENT_WHITE}
          />
        </MenubarTrigger>
        <MenubarContent label="Oxy" sideOffset={MENU_SIDE_OFFSET}>
          <MenubarItem onPress={() => command("system.settings")}>Ajustes del sistema</MenubarItem>
          <MenubarItem onPress={choose(launcher)}>Aplicaciones</MenubarItem>
          <MenubarItem onPress={choose(overview)}>Ver escritorios</MenubarItem>
          {canLock && (
            <MenubarItem onPress={() => command("session.lock")}>Bloquear pantalla</MenubarItem>
          )}
        </MenubarContent>
      </MenubarMenu>
      {menus.map((menu, index) => {
        const value = menuValue(index, menu);
        const weight = index === 0 ? ("600" as const) : ("400" as const);
        return (
          <MenubarMenu key={value} value={value}>
            <MenubarTrigger asChild label={menu.label} style={[{ borderRadius: 4 }, highlight(value)]}>
              <Button
                appearance="plain"
                className="h-7 rounded border-0 bg-transparent px-2"
                style={menuStyle}
                textStyle={{ ...menuText, fontWeight: weight }}
              >
                <Text style={{ ...menuText, fontWeight: weight }}>{menu.label}</Text>
              </Button>
            </MenubarTrigger>
            <MenubarContent label={menu.label} sideOffset={MENU_SIDE_OFFSET}>
              <MenuEntries items={menu.items} onCommand={command} />
            </MenubarContent>
          </MenubarMenu>
        );
      })}
    </Menubar>
  );
});

/**
 * The top bar: the menus, the battery and Wi-Fi, search, the Control Center and the clock. Each
 * part is memoized and renders again only when what it shows changes: the clock each minute, the
 * battery when the session says it changed, the menus when the active window's do.
 */
export const TopBar = memo(function TopBar({
  menus,
  now,
  overview,
  launcher,
  canLock = false,
  onCommand,
  power,
  network,
  controlCenter,
  clock,
  keyboardRequest = 0,
  keyboardLost = 0,
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
  /** The battery and Wi-Fi as the session says they are
      (docs/adr/0025-hardware-keys-and-control-center.mdx): only what it said is shown. */
  power?: ShellBattery;
  network?: ShellNetwork;
  /** Opens the Control Center, or closes it. */
  controlCenter: () => void;
  /** The clock opens and closes the Notification Center, as a Mac's (docs/adr/0026-notifications.mdx). */
  clock?: () => void;
  /** Rises at each Super+F2 (docs/adr/0024-system-shortcuts.mdx): the keyboard moves to the
      bar's first menu, as a Mac's Control-F2. */
  keyboardRequest?: number;
  /** Rises when the keyboard left the bar and its menus (a press in a window): the bar lets go. */
  keyboardLost?: number;
  /** Whether the bar wants the keyboard: its surface asks the platform for it. */
  onKeyboard?: (wanted: boolean) => void;
  /** Gives the keyboard back to the window that had it (the active one), as the bar is left. */
  returnKeyboard?: () => void;
}) {
  return (
    <Box
      testID="shell-topbar"
      className="h-7 shrink-0 flex-row items-center justify-between px-3"
      style={TOP_BAR_STYLE}
    >
      <BarMenus
        menus={menus}
        overview={overview}
        launcher={launcher}
        canLock={canLock}
        onCommand={onCommand}
        keyboardRequest={keyboardRequest}
        keyboardLost={keyboardLost}
        onKeyboard={onKeyboard}
        returnKeyboard={returnKeyboard}
      />
      <Box className="flex-row items-center gap-2.5">
        <Box className="flex-row items-center gap-4">
          {power && <BatteryItem power={power} />}
          {network && <WifiItem network={network} />}
        </Box>
        <BarButtons launcher={launcher} controlCenter={controlCenter} />
        <Clock now={now} onPress={clock} />
      </Box>
    </Box>
  );
});

