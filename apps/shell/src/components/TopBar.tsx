import React from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@oxy.so/bloom/dropdown-menu";
import { RiCheckboxBlankCircleLine } from "@oxy.so/bloom/icons/RiCheckboxBlankCircleLine";
import { RiWifiLine } from "@oxy.so/bloom/icons/RiWifiLine";
import { RiSearchLine } from "@oxy.so/bloom/icons/RiSearchLine";

import Svg, { Rect, Circle } from "react-native-svg";
import { Box } from "../primitives";
import type { ShellMenu } from "../model";

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
}: {
  menus: readonly ShellMenu[];
  now: Date;
  overview: () => void;
  launcher: () => void;
  /** Whether the session can be locked from here (the platform has a lock). */
  canLock?: boolean;
  onCommand: (id: string) => void;
}) {
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
              icon={RiCheckboxBlankCircleLine}
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
          <DropdownMenu key={menu.label}>
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
              {menu.items.map((item) => (
                <DropdownMenuItem
                  key={item.id}
                  disabled={item.enabled === false}
                  onPress={() => onCommand(item.id)}
                >
                  {item.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        ))}
      </Box>
      <Box className="flex-row items-center gap-2.5">
        <Box
          className="flex-row items-center gap-4"
          accessibilityLabel="Indicadores de ejemplo, sin conexión al sistema"
        >
          <Svg width={24} height={12} viewBox="0 0 26 13">
            <Rect
              x="1"
              y="1"
              width="21"
              height="11"
              rx="3"
              stroke="#ffffffcc"
              fill="none"
            />
            <Rect x="3" y="3" width="17" height="7" rx="1.5" fill="#fff" />
            <Rect
              x="23.5"
              y="4"
              width="1.5"
              height="5"
              rx=".7"
              fill="#ffffffb3"
            />
          </Svg>
          <RiWifiLine width={14} height={14} fill="#fff" />
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
          accessibilityLabel="Controles del sistema"
          appearance="plain"
          className="h-7 w-[22px] border-0 bg-transparent p-0"
          colors={{ background: "transparent", foreground: "#fff" }}
          onPress={() => onCommand("system.settings")}
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
