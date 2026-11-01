import React, { memo, useEffect, useRef, useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Switch } from "@oxy.so/bloom/switch";
import { SpinnerIcon } from "@oxy.so/bloom/loading";
import { SubtleHover } from "@oxy.so/bloom/subtle-hover";
import { RiArrowLeftSLine } from "@oxy.so/bloom/icons/RiArrowLeftSLine";
import { RiLockLine } from "@oxy.so/bloom/icons/RiLockLine";
import { Box, PressArea, Scroll } from "../primitives";
import type { ShellControls } from "../model";
import {
  isProtected,
  networkJoining,
  networkLabel,
  SCAN_INTERVAL_MS,
  signalArcs,
  wifiEmptyText,
  wifiSections,
  type NetworkState,
  type WifiNetwork,
} from "../network";
import { ACCENT, FACE, INK, QUIET, card } from "./control-style";
import { WifiArcs } from "./WifiArcs";

/** The lists scroll past this height, so the panel stays under the top bar on a small display. */
export const WIFI_LIST_MAX_HEIGHT = 360;
export const WIFI_ROW_HEIGHT = 36;

/** A network nearby: its signal in arcs on a round face (the accent's for the one joined), its
    name, a lock when it is protected, and progress while it is joined. Pressed, it is joined. */
function NetworkRow({
  network,
  state,
  join,
}: {
  network: WifiNetwork;
  state: NetworkState;
  join: (ssid: string) => void;
}) {
  const [hovered, setHovered] = useState(false);
  const busy = networkJoining(network, state);
  return (
    <PressArea
      testID={`wifi-network-${network.ssid}`}
      accessibilityRole="button"
      accessibilityLabel={networkLabel(network, state)}
      accessibilityState={{ selected: network.current, busy }}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      // The network joined is joined already: choosing it again does nothing.
      onPress={() => {
        if (!network.current) join(network.ssid);
      }}
      style={{
        height: WIFI_ROW_HEIGHT,
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        paddingHorizontal: 6,
        borderRadius: 8,
      }}
    >
      <SubtleHover native active={hovered} style={{ borderRadius: 8 }} />
      <Box
        className="items-center justify-center rounded-full"
        style={{ width: 26, height: 26, backgroundColor: network.current ? ACCENT : FACE }}
      >
        <WifiArcs
          arcs={signalArcs(network.signal)}
          color={network.current ? "#ffffff" : INK}
          faint={network.current ? "#ffffff59" : "#1d243040"}
        />
      </Box>
      <Text numberOfLines={1} style={{ flex: 1, color: INK, fontSize: 13 }}>
        {network.ssid}
      </Text>
      {busy && <SpinnerIcon size={14} color={QUIET} />}
      {isProtected(network) && <RiLockLine width={13} height={13} fill={QUIET} />}
    </PressArea>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Box style={{ ...card, paddingHorizontal: 6, paddingVertical: 8 }}>
      <Text
        accessibilityRole="header"
        style={{ color: QUIET, fontSize: 11, fontWeight: "600", paddingHorizontal: 6, marginBottom: 4 }}
      >
        {title}
      </Text>
      {children}
    </Box>
  );
}

/**
 * The Control Center's Wi-Fi view (docs/adr/0038-networks.mdx, decision 6), as a Mac's: in place
 * of the modules, Wi-Fi's switch; with Wi-Fi on, the known networks nearby (the one joined first)
 * and then the others, each with its signal and a lock when protected; "Ajustes de Wi-Fi…" at its
 * foot. It asks the session to scan as it opens and every ten seconds while open. Choosing a
 * network asks the session to join it; the session asks its password, or says why it cannot, in a
 * system sheet.
 */
export const WifiPanel = memo(function WifiPanel({
  state,
  controls,
  back,
  close,
}: {
  state: NetworkState;
  controls: ShellControls;
  /** Back to the modules. */
  back: () => void;
  /** Closes the Control Center (Settings opened). */
  close: () => void;
}) {
  const scan = useRef(controls.scanNetworks);
  scan.current = controls.scanNetworks;
  useEffect(() => {
    scan.current?.();
    const timer = setInterval(() => scan.current?.(), SCAN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, []);
  const join = (ssid: string) => controls.joinNetwork?.(ssid);
  const empty = wifiEmptyText(state);
  const { known, other } = wifiSections(state.wifi.networks);
  const row = (network: WifiNetwork) => (
    <NetworkRow key={network.ssid} network={network} state={state} join={join} />
  );
  return (
    <Box testID="control-wifi-detail" style={{ gap: 10 }}>
      <Box className="flex-row items-center" style={{ height: 32, gap: 6, paddingRight: 4 }}>
        <Button
          testID="wifi-back"
          iconOnly
          icon={RiArrowLeftSLine}
          iconSize={18}
          accessibilityLabel="Volver al centro de control"
          appearance="plain"
          className="border-0 p-0"
          style={{ width: 28, height: 28 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={back}
        />
        <Text accessibilityRole="header" style={{ flex: 1, color: INK, fontSize: 15, fontWeight: "600" }}>
          Wi-Fi
        </Text>
        <Switch
          testID="wifi-switch"
          size="sm"
          accessibilityLabel="Wi-Fi"
          checked={state.wifi.enabled}
          disabled={!state.available || state.wifi.hardwareBlocked || !controls.setWifi}
          onCheckedChange={(enabled) => controls.setWifi?.(enabled)}
        />
      </Box>
      {empty !== undefined ? (
        <Box style={{ ...card, alignItems: "center", paddingVertical: 18 }}>
          <Text testID="wifi-empty" style={{ color: QUIET, fontSize: 13, textAlign: "center" }}>
            {empty}
          </Text>
        </Box>
      ) : (
        <Scroll style={{ maxHeight: WIFI_LIST_MAX_HEIGHT }} contentContainerStyle={{ gap: 10 }}>
          {known.length > 0 && <Section title="Redes conocidas">{known.map(row)}</Section>}
          {other.length > 0 && <Section title="Otras redes">{other.map(row)}</Section>}
        </Scroll>
      )}
      {controls.openWifiSettings && (
        <Button
          testID="wifi-settings"
          appearance="plain"
          className="border-0"
          style={{ alignSelf: "flex-start", paddingHorizontal: 6, height: 26 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={() => {
            close();
            controls.openWifiSettings?.();
          }}
        >
          <Text style={{ color: INK, fontSize: 12 }}>Ajustes de Wi-Fi…</Text>
        </Button>
      )}
    </Box>
  );
});
