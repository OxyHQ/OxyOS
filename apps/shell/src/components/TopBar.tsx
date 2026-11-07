import React, { memo, useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
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
import { RiArrowLeftRightLine } from "@oxy.so/bloom/icons/RiArrowLeftRightLine";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuCheckboxItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@oxy.so/bloom/dropdown-menu";

import Svg, { Rect, Circle, Path } from "react-native-svg";
import { Box } from "../primitives";
import { WifiArcs } from "./WifiArcs";
import { OxyLogo } from "./OxyLogo";
import { batteryLabel, batteryMenuLines, significantApps, wifiLabel, wifiOn } from "../controls";
import { signalArcs } from "../network";
import type { ShellBattery, ShellBatteryMenu, ShellEnergyApp, ShellMenu, ShellMenuItem, ShellNetwork } from "../model";
import type { ShellKeyboard } from "../platform";
import { barClock, clockSaid } from "../clock";
import { useShellLocale, useShellMessages } from "../useShellMessages";
import { showsInputSources, sourceMark, sourceName, type InputSourcesState } from "../input-sources";

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

/** The battery as the session says it is: its charge filled in, a bolt while it charges; red when
    low, yellow in Low Power Mode, as a Mac's. */
function BatteryIcon({ power, lowPower = false }: { power: ShellBattery; lowPower?: boolean }) {
  const fill = Math.max(1, Math.round((17 * Math.min(100, power.percent)) / 100));
  const low = power.battery === "discharging" && power.percent <= 10;
  return (
    <Svg width={24} height={12} viewBox="0 0 26 13">
      <Rect x="1" y="1" width="21" height="11" rx="3" stroke="#ffffffcc" fill="none" />
      <Rect x="3" y="3" width={fill} height="7" rx="1.5" fill={low ? "#ff5a4f" : lowPower ? "#ffd60a" : "#fff"} />
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
  return (
    <WifiArcs
      arcs={connected ? signalArcs(network.signal) : 0}
      dot={connected || wifiOn(network)}
      color="#fff"
      faint="#ffffff59"
    />
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
  const messages = useShellMessages();
  const battery = batteryLabel(power, messages);
  if (battery === undefined) return null;
  // A level indicator, as VoiceOver reads a battery's charge: named "Batería", its charge and
  // what it is doing its value ("Batería, 64 %, 3 h 20 min restantes, indicador de nivel").
  return (
    <Box
      testID="shell-battery"
      accessible
      role="meter"
      accessibilityLabel={messages.topBar.battery}
      accessibilityValue={{ min: 0, max: 100, now: power.percent, text: battery }}
    >
      <BatteryIcon power={power} />
    </Box>
  );
});

const BATTERY_ITEM = "shell-battery";
const BATTERY_MENU_WIDTH = 260;
const batteryTitle = { fontWeight: "600" } as const;
const percentText = { color: "#fff", fontSize: 12, lineHeight: 16, marginRight: 4 } as const;

/**
 * The battery's status menu (docs/adr/0057-battery-and-energy.mdx), as a Mac's: the icon (and its
 * charge in percent, as the person chose), opening the charge, the power source and the time left
 * or until full, the applications using significant energy (sampled as it opens), and Battery
 * Settings…. Named "Battery", its charge and state its value, as the level indicator was.
 */
const BatteryMenuItem = memo(function BatteryMenuItem({
  power,
  menu,
  onItemFocus,
  onOpenChange,
  restoreFocus,
}: {
  power: ShellBattery;
  menu: ShellBatteryMenu;
  onItemFocus?: (testID: string) => void;
  onOpenChange?: (open: boolean) => void;
  restoreFocus: boolean;
}) {
  const messages = useShellMessages();
  const [open, setOpenState] = useState(false);
  // The applications' energy, as the session sampled it when the menu opened; null until it says.
  const [apps, setApps] = useState<readonly ShellEnergyApp[] | null>(null);
  const { energy } = menu;
  const setOpen = useCallback(
    (next: boolean) => {
      setOpenState(next);
      onOpenChange?.(next);
      if (!next) return;
      setApps(null);
      energy().then(setApps, () => setApps([]));
    },
    [onOpenChange, energy],
  );
  const lines = batteryMenuLines(power, messages);
  const said = batteryLabel(power, messages);
  if (!lines || said === undefined) return null;
  const significant = apps === null ? null : significantApps(apps);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild restoreFocus={restoreFocus}>
        <Button
          appearance="plain"
          testID={BATTERY_ITEM}
          accessibilityLabel={messages.topBar.battery}
          accessibilityValue={{ min: 0, max: 100, now: power.percent, text: said }}
          aria-expanded={open}
          style={{ ...menuStyle, paddingLeft: 2, paddingRight: 2 }}
          colors={TRANSPARENT_WHITE}
          hitSlop={STATUS_SLOP}
          onFocus={() => onItemFocus?.(BATTERY_ITEM)}
        >
          <Box className="flex-row items-center">
            {menu.showPercentage && <Text style={percentText}>{lines.charge}</Text>}
            <BatteryIcon power={power} lowPower={menu.lowPower} />
          </Box>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent label={messages.battery.title} sideOffset={MENU_SIDE_OFFSET} minWidth={BATTERY_MENU_WIDTH}>
        <DropdownMenuLabel style={batteryTitle}>{`${messages.battery.title}  ${lines.charge}`}</DropdownMenuLabel>
        <DropdownMenuLabel>{lines.source}</DropdownMenuLabel>
        <DropdownMenuLabel>{lines.time}</DropdownMenuLabel>
        {menu.lowPower && <DropdownMenuLabel>{messages.battery.lowPower}</DropdownMenuLabel>}
        {significant !== null && (
          <>
            <DropdownMenuSeparator />
            {significant.length === 0 ? (
              <DropdownMenuLabel>{messages.battery.noSignificant}</DropdownMenuLabel>
            ) : (
              <>
                <DropdownMenuLabel style={batteryTitle}>{messages.battery.significant}</DropdownMenuLabel>
                {significant.map((app) => (
                  <DropdownMenuLabel key={app.id} inset>
                    {app.name}
                  </DropdownMenuLabel>
                ))}
              </>
            )}
          </>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem testID="shell-battery-settings" onPress={menu.openSettings}>
          {messages.battery.settings}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
});

/** The Ethernet icon, as Bloom's icons are drawn (a Button's `icon`). */
function EthernetIcon({ width = 15, height = 15, fill = "#fff" }: { width?: number; height?: number; fill?: string }) {
  return <RiArrowLeftRightLine width={width} height={height} fill={fill} />;
}

/** Wi-Fi's icons as a Button's `icon`, one per arcs lit and dot, made once each. */
const wifiIcons = new Map<string, ComponentType>();
function wifiIcon(arcs: number, dot: boolean): ComponentType {
  const key = `${arcs}:${dot}`;
  let icon = wifiIcons.get(key);
  if (!icon) {
    icon = function WifiStatusIcon() {
      return <WifiArcs arcs={arcs} dot={dot} color="#fff" faint="#ffffff59" />;
    };
    wifiIcons.set(key, icon);
  }
  return icon;
}

/** A status item's button: the bar's own plain icon button, as the search and the Control Center's,
    as wide as its icon (the items keep their places) and pressed anywhere over the bar's height
    around it. */
const statusStyle = { ...menuStyle, width: 14, paddingLeft: 0, paddingRight: 0 };
const STATUS_SLOP = { top: 8, bottom: 8, left: 6, right: 6 };

/** The network's status item (docs/adr/0038-networks.mdx, decision 6): Wi-Fi's signal, or the
    Ethernet icon while only a cable is connected. Pressed, it opens the Control Center at its
    Wi-Fi view, where the platform gives `onPress`. */
/** The network item's testID where it is a button, as `NetworkItem` names it; none where it
    shows nothing. */
function networkItem(network: ShellNetwork | undefined): string | undefined {
  if (!network) return undefined;
  const ethernet = network.wifi !== "connected" && network.wired;
  if (ethernet) return "shell-ethernet";
  return network.wifi === "unavailable" ? undefined : "shell-wifi";
}

/** Which panels the status items open are shown: the network's (Control Center at its Wi-Fi
    view), Control Center (at any view), the Notification Center. */
export interface StatusPanels {
  network: boolean;
  controlCenter: boolean;
  notifications: boolean;
}
const NOTHING_OPEN: StatusPanels = { network: false, controlCenter: false, notifications: false };

/** The status items, as the keyboard goes along them (← →, docs/adr/0039-keyboard-access.mdx): the
    battery's first where it is a menu. */
function statusItems(
  network: ShellNetwork | undefined,
  inputSources?: InputSourcesState | null,
  battery = false,
): string[] {
  const first = networkItem(network);
  return [
    ...(battery ? [BATTERY_ITEM] : []),
    ...(first ? [first] : []),
    ...(showsInputSources(inputSources) ? [INPUT_SOURCE_ITEM] : []),
    "shell-search",
    "shell-control-center",
    "shell-clock",
  ];
}

/** The status item Super+F8 puts the keyboard on, as a Mac's Control-F8 its first status menu. */
export function firstStatusItem(
  network: ShellNetwork | undefined,
  inputSources?: InputSourcesState | null,
  battery = false,
): string {
  return statusItems(network, inputSources, battery)[0];
}

const INPUT_SOURCE_ITEM = "shell-input-source";
/** The input source's mark, as a Mac's Input menu draws a source with no flag: its letters or
    character in a rounded outline, white over the wallpaper. */
const sourceMarkBox = {
  minWidth: 20,
  height: 16,
  paddingHorizontal: 3,
  borderRadius: 4,
  borderWidth: 1,
  borderColor: "#ffffffd9",
  alignItems: "center",
  justifyContent: "center",
} as const;
const sourceMarkText = { color: "#fff", fontSize: 11, lineHeight: 14, fontWeight: "600" } as const;
const INPUT_MENU_WIDTH = 248;

/**
 * The input source's status item (docs/adr/0041-input-methods.mdx, decision 6), as a Mac's Input
 * menu: shown only with more than one source, it shows the current one (`ES`, 拼, あ, 한) and opens
 * a menu of the sources, the current one checked, then "Show Emoji & Symbols" and "Open Keyboard
 * Settings…". Named by what it is, the source in effect its value ("Input source, Pinyin –
 * Simplified"), as the Wi-Fi item is named.
 */
const InputSourceItem = memo(function InputSourceItem({
  state,
  onSelect,
  onShowEmoji,
  onOpenSettings,
  onItemFocus,
  onOpenChange,
  restoreFocus,
}: {
  state: InputSourcesState;
  onSelect?: (id: string) => void;
  onShowEmoji?: () => void;
  onOpenSettings?: () => void;
  onItemFocus?: (testID: string) => void;
  /** Its menu opened or closed. */
  onOpenChange?: (open: boolean) => void;
  /** Whether the screen reader's focus comes back to the item as its menu closes: opened from the
      status items' keys, yes; opened with the pointer, the keyboard goes back to the window
      instead (`returnKeyboard`), which a restore 50 ms later would take back. */
  restoreFocus: boolean;
}) {
  const messages = useShellMessages();
  const [open, setOpenState] = useState(false);
  const setOpen = useCallback(
    (next: boolean) => {
      setOpenState(next);
      onOpenChange?.(next);
    },
    [onOpenChange],
  );
  const current = state.sources.includes(state.current) ? state.current : "layout";
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild restoreFocus={restoreFocus}>
        <Button
          appearance="plain"
          testID={INPUT_SOURCE_ITEM}
          accessibilityLabel={messages.inputSources.item}
          accessibilityValue={{ text: sourceName(current, state, messages) }}
          aria-expanded={open}
          style={{ ...menuStyle, paddingLeft: 2, paddingRight: 2 }}
          colors={TRANSPARENT_WHITE}
          hitSlop={STATUS_SLOP}
          onFocus={() => onItemFocus?.(INPUT_SOURCE_ITEM)}
        >
          <Box style={sourceMarkBox}>
            <Text style={sourceMarkText}>{sourceMark(current, state)}</Text>
          </Box>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent label={messages.inputSources.item} sideOffset={MENU_SIDE_OFFSET} minWidth={INPUT_MENU_WIDTH}>
        {state.sources.map((id) => (
          <DropdownMenuCheckboxItem key={id} checked={id === current} onCheckedChange={() => onSelect?.(id)}>
            {sourceName(id, state, messages)}
          </DropdownMenuCheckboxItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onPress={() => onShowEmoji?.()}>{messages.inputSources.showEmoji}</DropdownMenuItem>
        <DropdownMenuItem onPress={() => onOpenSettings?.()}>{messages.inputSources.openKeyboardSettings}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
});

const NetworkItem = memo(function NetworkItem({
  network,
  open,
  onPress,
  onItemFocus,
}: {
  network: ShellNetwork;
  /** Whether what it opens (Control Center's Wi-Fi view) is shown. */
  open: boolean;
  onPress?: () => void;
  onItemFocus?: (testID: string) => void;
}) {
  const messages = useShellMessages();
  const ethernet = network.wifi !== "connected" && network.wired;
  const wifi = ethernet ? undefined : wifiLabel(network, messages);
  if (!ethernet && wifi === undefined) return null;
  // Named by what it is, the network it is on its value, as VoiceOver says a Mac's Wi-Fi menu
  // ("Wi-Fi, Vecinos").
  const label = ethernet ? "Ethernet" : "Wi-Fi";
  const value = { text: ethernet ? messages.topBar.connected : wifi };
  const testID = ethernet ? "shell-ethernet" : "shell-wifi";
  if (!onPress) {
    return (
      <Box testID={testID} accessible accessibilityLabel={label} accessibilityValue={value}>
        {ethernet ? <EthernetIcon /> : <WifiIcon network={network} />}
      </Box>
    );
  }
  const connected = network.wifi === "connected";
  return (
    <Button
      iconOnly
      icon={ethernet ? EthernetIcon : wifiIcon(connected ? signalArcs(network.signal) : 0, connected || wifiOn(network))}
      iconSize={15}
      testID={testID}
      accessibilityLabel={label}
      accessibilityValue={value}
      // A status menu's button says whether its menu is open, as a Mac's does.
      aria-expanded={open}
      appearance="plain"
      style={statusStyle}
      colors={TRANSPARENT_WHITE}
      hitSlop={STATUS_SLOP}
      onPress={onPress}
      onFocus={() => onItemFocus?.(testID)}
    />
  );
});

const TOP_BAR_STYLE = { zIndex: 30 };
const TRANSPARENT_WHITE = { background: "transparent", foreground: "#fff" };
const searchStyle = { ...menuStyle, width: 22, paddingHorizontal: 0 };
const clockStyle = { ...menuStyle, paddingLeft: 4, paddingRight: 0 };

/** The search and the Control Center's buttons. */
const BarButtons = memo(function BarButtons({
  launcher,
  controlCenter,
  controlCenterOpen,
  onItemFocus,
}: {
  launcher: () => void;
  controlCenter: () => void;
  /** Whether the Control Center is shown. */
  controlCenterOpen: boolean;
  onItemFocus?: (testID: string) => void;
}) {
  const messages = useShellMessages();
  return (
    <>
      <Button
        iconOnly
        icon={RiSearchLine}
        testID="shell-search"
        accessibilityLabel={messages.common.searchApplications}
        appearance="plain"
        style={searchStyle}
        iconSize={13}
        colors={TRANSPARENT_WHITE}
        onPress={launcher}
        onFocus={() => onItemFocus?.("shell-search")}
      />
      <Button
        iconOnly
        icon={ControlsIcon}
        iconSize={15}
        testID="shell-control-center"
        accessibilityLabel={messages.common.controlCenter}
        aria-expanded={controlCenterOpen}
        appearance="plain"
        className="h-7 w-[22px] border-0 bg-transparent p-0"
        colors={TRANSPARENT_WHITE}
        onPress={controlCenter}
        onFocus={() => onItemFocus?.("shell-control-center")}
      />
    </>
  );
});

/** The clock: the date and the time, which open the Notification Center. As VoiceOver says a
    Mac's menu bar clock, it is named by what it shows, in whole words ("lunes, 3 de noviembre,
    14:05", "Tuesday, November 3, 2:05 PM"): the date and time are what it is. What pressing it
    does, which its name does not say, is its hint, said after it; it says whether the Notification
    Center is open as a status menu's button does. */
const Clock = memo(function Clock({
  now,
  open,
  onPress,
  onItemFocus,
}: {
  now: Date;
  /** Whether the Notification Center is shown. */
  open: boolean;
  onPress?: () => void;
  onItemFocus?: (testID: string) => void;
}) {
  const messages = useShellMessages();
  const locale = useShellLocale();
  const shown = barClock(now, locale);
  return (
    <Button
      testID="shell-clock"
      appearance="plain"
      accessibilityLabel={clockSaid(now, locale)}
      accessibilityHint={onPress ? messages.topBar.clockHint : undefined}
      aria-expanded={onPress ? open : undefined}
      onFocus={() => onItemFocus?.("shell-clock")}
      // Where the clock's text always was (its old `ml-1`), now pressable.
      style={clockStyle}
      onPress={onPress}
    >
      <Text className="text-[12px] leading-4 font-normal text-white">
        {shown}
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
  const messages = useShellMessages();
  const highlight = (value: string) =>
    focused === value ? { backgroundColor: keyboardHighlight } : undefined;
  return (
    <Menubar
      label={messages.topBar.menuBar}
      value={open}
      onValueChange={setOpen}
      focusValue={focused}
      onFocusValueChange={(value) => (value === undefined ? leave(true) : setFocused(value))}
      style={barStyle}
    >
      <MenubarMenu value={OXY}>
        <MenubarTrigger asChild label={messages.topBar.oxyMenu} style={[{ borderRadius: 4 }, highlight(OXY)]}>
          <Button
            iconOnly
            icon={OxyLogo}
            accessibilityLabel={messages.topBar.oxyMenu}
            appearance="plain"
            className="h-7 w-8 border-0 bg-transparent p-0"
            iconSize={17}
            colors={TRANSPARENT_WHITE}
          />
        </MenubarTrigger>
        <MenubarContent label="Oxy" sideOffset={MENU_SIDE_OFFSET}>
          <MenubarItem onPress={() => command("system.settings")}>{messages.common.systemSettings}</MenubarItem>
          <MenubarItem onPress={choose(launcher)}>{messages.topBar.applications}</MenubarItem>
          <MenubarItem onPress={choose(overview)}>{messages.common.showDesktops}</MenubarItem>
          {canLock && (
            <MenubarItem onPress={() => command("session.lock")}>{messages.topBar.lockScreen}</MenubarItem>
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
                <Text testID={`shell-menu-${menu.label}`} style={{ ...menuText, fontWeight: weight }}>
                  {menu.label}
                </Text>
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
  networkStatus,
  clock,
  open = NOTHING_OPEN,
  keyboardRequest = 0,
  keyboardLost = 0,
  onKeyboard,
  returnKeyboard,
  statusKeys,
  moveStatusFocus,
  leaveStatus,
  inputSources,
  selectInputSource,
  showEmoji,
  openKeyboardSettings,
  onInputMenuChange,
  batteryMenu,
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
  /** The network's status item pressed: the Control Center at its Wi-Fi view, or closed. */
  networkStatus?: () => void;
  /** The clock opens and closes the Notification Center, as a Mac's (docs/adr/0026-notifications.mdx). */
  clock?: () => void;
  /** Which of the panels the status items open is shown: each item says it is expanded while
      its own is (docs/adr/0039-keyboard-access.mdx). */
  open?: StatusPanels;
  /** Rises at each Super+F2 (docs/adr/0024-system-shortcuts.mdx): the keyboard moves to the
      bar's first menu, as a Mac's Control-F2. */
  keyboardRequest?: number;
  /** Rises when the keyboard left the bar and its menus (a press in a window): the bar lets go. */
  keyboardLost?: number;
  /** Whether the bar wants the keyboard: its surface asks the platform for it. */
  onKeyboard?: (wanted: boolean) => void;
  /** Gives the keyboard back to the window that had it (the active one), as the bar is left. */
  returnKeyboard?: () => void;
  /** The bar's keys while the keyboard is on its status items (Super+F8): ← → go along them,
      Escape leaves them; Tab and Space are the platform's. */
  statusKeys?: ShellKeyboard;
  moveStatusFocus?: (testID: string) => void;
  leaveStatus?: () => void;
  /** The input sources as the session tells them (docs/adr/0041-input-methods.mdx): the item is
      shown with more than one. A source chosen, the emoji picker and Keyboard settings asked for. */
  inputSources?: InputSourcesState | null;
  selectInputSource?: (id: string) => void;
  showEmoji?: () => void;
  openKeyboardSettings?: () => void;
  /** The input source's or the battery's menu opened or closed: while open, its keys (Escape
      among them) are its own. */
  onInputMenuChange?: (open: boolean) => void;
  /** The battery's menu, once the session told what it needs (docs/adr/0057-battery-and-
      energy.mdx); without it the battery is a level indicator alone. */
  batteryMenu?: ShellBatteryMenu;
}) {
  // The status item the keyboard is on, as the platform focuses it.
  const focusedItem = useRef<string | null>(null);
  // Whether the input source's menu is open: its keys are its own (its rows, Escape), not the
  // status items'.
  const inputMenuOpen = useRef(false);
  const batteryButton = !!(power && batteryMenu && power.battery !== "absent");
  const items = statusItems(network, inputSources, batteryButton);
  const latestItems = useRef(items);
  latestItems.current = items;
  useEffect(() => {
    if (!statusKeys) {
      focusedItem.current = null;
      return undefined;
    }
    return statusKeys.subscribe((event) => {
      if (inputMenuOpen.current) return false;
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
      if (event.key === "Escape") {
        leaveStatus?.();
        return true;
      }
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return false;
      const all = latestItems.current;
      const at = focusedItem.current ? all.indexOf(focusedItem.current) : -1;
      const step = event.key === "ArrowRight" ? 1 : -1;
      const next = all[(Math.max(at, 0) + step + all.length) % all.length];
      if (next) moveStatusFocus?.(next);
      return true;
    });
  }, [statusKeys, moveStatusFocus, leaveStatus]);
  const onItemFocus = useCallback((testID: string) => {
    focusedItem.current = testID;
  }, []);
  // The input source's menu closed: opened with the pointer, the keyboard goes back to the window
  // that had it, as a Mac's Input menu leaves it; from the status items (Super+F8) it stays on the
  // item, as Escape from Control Center leaves it.
  const fromKeyboard = useRef(!!statusKeys);
  fromKeyboard.current = !!statusKeys;
  const inputMenuChanged = useCallback(
    (open: boolean) => {
      inputMenuOpen.current = open;
      onInputMenuChange?.(open);
      if (!open && !fromKeyboard.current) returnKeyboard?.();
    },
    [returnKeyboard, onInputMenuChange],
  );
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
          {power &&
            (batteryMenu ? (
              <BatteryMenuItem
                power={power}
                menu={batteryMenu}
                onItemFocus={onItemFocus}
                onOpenChange={inputMenuChanged}
                restoreFocus={!!statusKeys}
              />
            ) : (
              <BatteryItem power={power} />
            ))}
          {network && (
            <NetworkItem network={network} open={open.network} onPress={networkStatus} onItemFocus={onItemFocus} />
          )}
          {showsInputSources(inputSources) && (
            <InputSourceItem
              state={inputSources}
              onSelect={selectInputSource}
              onShowEmoji={showEmoji}
              onOpenSettings={openKeyboardSettings}
              onItemFocus={onItemFocus}
              onOpenChange={inputMenuChanged}
              restoreFocus={!!statusKeys}
            />
          )}
        </Box>
        <BarButtons
          launcher={launcher}
          controlCenter={controlCenter}
          controlCenterOpen={open.controlCenter}
          onItemFocus={onItemFocus}
        />
        <Clock now={now} open={open.notifications} onPress={clock} onItemFocus={onItemFocus} />
      </Box>
    </Box>
  );
});

