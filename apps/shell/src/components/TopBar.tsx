import React from "react";
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
export function TopBar({
  menus,
  now,
  overview,
  launcher,
  canLock = false,
  onCommand,
  controls,
  controlCenter,
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
}) {
  const battery = batteryLabel(controls?.power);
  const wifi = wifiLabel(controls?.network);
  return (
    <Box
      testID="shell-topbar"
      className="h-7 shrink-0 flex-row items-center justify-between px-3"
      style={{ zIndex: 30 }}
    >
      <Box className="flex-row items-center gap-0">
        <DropdownMenu>
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
          <DropdownMenuContent
            align="start"
            label="Oxy"
            size="sm"
            minWidth={220}
            className="rounded-lg"
            style={{ borderRadius: 8 }}
          >
            <DropdownMenuItem onPress={() => onCommand("system.settings")}>
              Ajustes del sistema
            </DropdownMenuItem>
            <DropdownMenuItem onPress={launcher}>Aplicaciones</DropdownMenuItem>
            <DropdownMenuItem onPress={overview}>
              Ver escritorios
            </DropdownMenuItem>
            {canLock && (
              <DropdownMenuItem onPress={() => onCommand("session.lock")}>
                Bloquear pantalla
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        {menus.map((menu, index) => (
          <DropdownMenu key={`${index}:${menu.label}`}>
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
            <DropdownMenuContent
              align="start"
              label={menu.label}
              size="sm"
              minWidth={220}
              className="rounded-lg"
              style={{ borderRadius: 8 }}
            >
              <MenuEntries items={menu.items} onCommand={onCommand} />
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
        <Text className="ml-1 text-[12px] leading-4 font-normal text-white">
          {now.toLocaleDateString("es", {
            weekday: "short",
            day: "numeric",
            month: "short",
          })}{" "}
          {now.toLocaleTimeString("es", { hour: "2-digit", minute: "2-digit" })}
        </Text>
      </Box>
    </Box>
  );
}
