/**
 * Settings › Wi-Fi and › Network (docs/adr/0038-networks.mdx), as a Mac's: the network's state as
 * the session tells it and Settings' requests of it. Settings never asks or sees a password (the
 * session asks it in the shell's sheet) and keeps nothing of its own; NetworkManager has it all.
 *
 * Since 8 November 2026: each known network's sheet (Details… for the one joined, Network
 * Settings… in a known network's menu), as a Mac's, with its Private Wi-Fi Address (Off, Fixed,
 * Rotating; changing it on the network joined rejoins it, said there), the address in use and
 * Forget This Network…; Wi-Fi Diagnostics, run once when opened and again only when asked; and
 * Network's Reset Network Settings…, whose confirmation lists what goes and what is kept.
 */
import * as React from "react";
import { AccessibilityInfo, View } from "react-native";
import Svg, { Circle, Path } from "react-native-svg";
import { AlertDialog } from "@oxy.so/bloom/alert-dialog";
import { Button } from "@oxy.so/bloom/button";
import { Dialog } from "@oxy.so/bloom/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@oxy.so/bloom/dropdown-menu";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { RiGlobalLine, RiLockLine, RiMoreLine, RiWifiLine } from "@oxy.so/bloom/icons";
import { Loading } from "@oxy.so/bloom/loading";
import {
  SegmentedControl,
  SegmentedControlItem,
  SegmentedControlItemText,
} from "@oxy.so/bloom/segmented-control";
import { SettingsSelect } from "@oxy.so/bloom/settings-modal";
import { Switch } from "@oxy.so/bloom/switch";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  currentNetwork,
  detailRows,
  diagnosticRows,
  ethernetStatus,
  failureLabel,
  failureOf,
  formatSpeed,
  isJoined,
  isLocked,
  joinFailureNeedsNote,
  knownByName,
  otherNetworks,
  PRIVATE_ADDRESS_CHOICES,
  privateAddressFailure,
  privateAddressOutcome,
  privateChoice,
  resetLines,
  resetFailureOf,
  resetPlanOf,
  SCAN_INTERVAL_MS,
  type Diagnostics,
  type KnownNetwork,
  type NearbyNetwork,
  type NetworkDetails,
  type PrivateAddressChoice,
  type ResetPlan,
  type WifiState,
} from "./network";
// The platform's one rule for a signal's arcs, the shell's (and the session's).
import { signalArcs } from "../../shell/src/network";
import { messagesLanguage, type Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";

type PageProps = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };
type Words = Messages["wifi"];

const PAGE = { gap: 20 } as const;
const TRAILING = { flexDirection: "row", alignItems: "center", gap: 12 } as const;

/** A network's signal in three arcs, as the shell's top bar draws it; the arcs it does not reach
    faint. Its words are its row's. */
function SignalGlyph({ signal }: { signal: number }) {
  const theme = useTheme();
  const bars = signalArcs(signal);
  const arc = (on: boolean) => (on ? theme.colors.text : theme.colors.textTertiary);
  return (
    <Svg width={18} height={14} viewBox="0 0 20 16">
      <Path d="M1 5.6a13 13 0 0 1 18 0" stroke={arc(bars >= 3)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M4.2 8.9a8.4 8.4 0 0 1 11.6 0" stroke={arc(bars >= 2)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Path d="M7.4 12.1a3.9 3.9 0 0 1 5.2 0" stroke={arc(bars >= 1)} strokeWidth="2" strokeLinecap="round" fill="none" />
      <Circle cx="10" cy="14.6" r="1.3" fill={arc(bars >= 1)} />
    </Svg>
  );
}

/** At a network's row's end, as a Mac lists networks: a lock when joining asks for a password,
    then its signal; neither for a known network not nearby. */
function NetworkGlyphs({ network }: { network: NearbyNetwork | undefined }) {
  const theme = useTheme();
  if (!network) return null;
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      {isLocked(network.security) ? <RiLockLine size="sm" fill={theme.colors.textSecondary} /> : null}
      <SignalGlyph signal={network.signal} />
    </View>
  );
}

/** What a network's row says to the screen reader: its name, its state, its lock and signal. */
function networkLabel(
  ssid: string,
  network: NearbyNetwork | undefined,
  words: Words,
  ...states: (string | null | undefined)[]
): string {
  const glyphs = network
    ? [isLocked(network.security) ? words.secured : words.open, words.signal(signalArcs(network.signal))]
    : [];
  return [ssid, ...states, ...glyphs].filter(Boolean).join(", ");
}

/** A connection's details (ADR 0038, decision 7), one row each, only those it has; its hardware
    address left out where the sheet says it beside the private address. */
function detailItems(
  details: NetworkDetails | null,
  strings: Messages,
  hardwareLabel: string | null,
  prefix: string,
): React.ReactNode[] {
  const labels = { ...strings.details, hardwareAddress: hardwareLabel ?? "" };
  return detailRows(details)
    .filter((row) => hardwareLabel !== null || row.key !== "hardwareAddress")
    .map((row) => (
      <GroupedSectionItem key={row.key} title={labels[row.key]} value={row.value} testID={`${prefix}-${row.key}`} />
    ));
}

/** A dialog's content kept while it closes, so its words do not vanish under its animation. */
function useHeld<T>(): [T | null, boolean, (value: T) => void, () => void] {
  const [held, setHeld] = React.useState<{ value: T | null; open: boolean }>({ value: null, open: false });
  const show = React.useCallback((value: T) => setHeld({ value, open: true }), []);
  const hide = React.useCallback(() => setHeld((current) => ({ ...current, open: false })), []);
  return [held.value, held.open, show, hide];
}

/** A row that only says something: a page with nothing else to show. */
function Notice({ title, testID }: { title: string; testID: string }) {
  return (
    <GroupedSection>
      <GroupedSectionItem title={title} titleNumberOfLines={0} testID={testID} />
    </GroupedSection>
  );
}

/**
 * A known network's sheet, as a Mac's Details: its Wi-Fi part (the Private Wi-Fi Address, chosen
 * among Off, Fixed and Rotating, one set elsewhere showing none chosen; for the network joined, the
 * address in use and that changing it rejoins; then Forget This Network…) and, for the network
 * joined, its TCP/IP part (its addresses, router and DNS), each a tab as a Mac's sheet has them in
 * its sidebar. The session sets the address on NetworkManager's connection and rejoins; the state
 * it then tells is what the sheet shows.
 */
function NetworkSheet({
  ssid,
  open,
  wifi,
  strings,
  setting,
  onClose,
  onChoose,
  onForget,
}: {
  ssid: string;
  open: boolean;
  wifi: WifiState;
  strings: Messages;
  /** The choice being set, while the session sets it. */
  setting: PrivateAddressChoice | null;
  onClose: () => void;
  onChoose: (known: KnownNetwork, mode: PrivateAddressChoice) => void;
  onForget: (known: KnownNetwork) => void;
}) {
  const words = strings.wifi;
  const known = knownByName(wifi, ssid);
  const joined = known ? isJoined(wifi, known) : false;
  const chosen = setting ?? (known ? privateChoice(known) : null);
  const footer = joined ? `${words.privateFooter} ${words.privateRejoins(ssid)}` : words.privateFooter;
  // Each time it opens, at its Wi-Fi part, as a Mac's.
  const [tab, setTab] = React.useState<"wifi" | "tcpip">("wifi");
  React.useEffect(() => {
    if (open) setTab("wifi");
  }, [open, ssid]);
  const shown = joined ? tab : "wifi";
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={ssid}
      label={words.detailsLabel(ssid)}
      actions={[{ label: words.ok, color: "default", onPress: onClose, testID: "wifi-details-ok" }]}
      testID="wifi-details-sheet"
    >
      <View style={{ gap: 16 }}>
        {joined ? (
          <SegmentedControl<"wifi" | "tcpip">
            label={words.detailsLabel(ssid)}
            type="tabs"
            value={shown}
            onValueChange={setTab}
            testID="wifi-details-tabs"
          >
            {(["wifi", "tcpip"] as const).map((part) => (
              <SegmentedControlItem key={part} value={part} testID={`wifi-details-tab-${part}`}>
                <SegmentedControlItemText>{words.sheetTabs[part]}</SegmentedControlItemText>
              </SegmentedControlItem>
            ))}
          </SegmentedControl>
        ) : null}
        {known && shown === "wifi" ? (
          <GroupedSection footer={footer}>
            <GroupedSectionItem
              title={words.privateAddress}
              testID="wifi-private"
              rightElement={
                <SettingsSelect
                  label={words.privateAddress}
                  value={chosen ?? ""}
                  items={PRIVATE_ADDRESS_CHOICES.map((mode) => ({ value: mode, label: words.privateModes[mode] }))}
                  // Never disabled while the session sets it, so the keyboard comes back to it as
                  // its choices close; a choice made meanwhile waits for none.
                  onChange={(value) => {
                    const mode = PRIVATE_ADDRESS_CHOICES.find((choice) => choice === value);
                    if (mode && mode !== chosen && setting === null) onChoose(known, mode);
                  }}
                  testID="wifi-private-address"
                />
              }
            />
            {known.privateAddress === "other" ? (
              <GroupedSectionItem title={words.privateModes.other} titleNumberOfLines={0} testID="wifi-private-other" />
            ) : null}
            {joined && wifi.details?.hardwareAddress ? (
              <GroupedSectionItem
                title={words.wifiAddress}
                value={wifi.details.hardwareAddress}
                testID="wifi-details-hardwareAddress"
              />
            ) : null}
          </GroupedSection>
        ) : null}
        {shown === "tcpip" ? (
          <GroupedSection>{detailItems(wifi.details, strings, null, "wifi-details")}</GroupedSection>
        ) : null}
        {known && shown === "wifi" ? (
          <GroupedSection>
            <GroupedSectionItem
              title={words.forgetItem}
              destructive
              showChevron={false}
              accessibilityRole="button"
              onPress={() => onForget(known)}
              testID="wifi-details-forget"
            />
          </GroupedSection>
        ) : null}
      </View>
    </Dialog>
  );
}

/**
 * Wi-Fi Diagnostics: asked of the session once as the sheet opens and again only on Check Again;
 * nothing runs while it stays open or after it closed. What it shows is what was measured, each
 * check with its plain verdict.
 */
function DiagnosticsSheet({
  open,
  run,
  strings,
  locale,
  onClose,
}: {
  open: boolean;
  run: () => Promise<{ found: Diagnostics } | { failure: string }>;
  strings: Messages;
  locale: string;
  onClose: () => void;
}) {
  const words = strings.wifi.diagnostics;
  const language = messagesLanguage(locale);
  const [outcome, setOutcome] = React.useState<{ found: Diagnostics } | { failure: string } | null>(null);
  const [running, setRunning] = React.useState(false);
  const turn = React.useRef(0);
  const check = React.useCallback(async () => {
    const mine = ++turn.current;
    setRunning(true);
    const told = await run();
    if (mine !== turn.current) return;
    setOutcome(told);
    setRunning(false);
    if ("found" in told) {
      // Its verdicts said once, as they come: the sheet's own rows say the rest.
      const said = diagnosticRows(told.found, words, language)
        .filter((row) => row.key !== "connection")
        .map((row) => `${row.title}: ${row.value}`);
      AccessibilityInfo.announceForAccessibility(said.join(". "));
    }
  }, [run, words, language]);
  // Once as it opens; a closed sheet forgets what it found, and an answer that comes after is
  // dropped.
  React.useEffect(() => {
    if (!open) {
      turn.current += 1;
      return;
    }
    setOutcome(null);
    void check();
  }, [open]);
  const rows = outcome && "found" in outcome ? diagnosticRows(outcome.found, words, language) : [];
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={words.title}
      label={words.title}
      actions={[
        {
          label: words.again,
          color: "cancel",
          loading: running,
          shouldCloseOnPress: false,
          onPress: () => {
            void check();
          },
          testID: "wifi-diagnostics-again",
        },
        { label: words.done, color: "default", onPress: onClose, testID: "wifi-diagnostics-done" },
      ]}
      testID="wifi-diagnostics-sheet"
    >
      {running || outcome === null ? (
        <View style={{ alignItems: "center", gap: 12, paddingVertical: 24 }}>
          <Loading size="sm" accessibilityLabel={words.checking} testID="wifi-diagnostics-checking" />
          <Text variant="body-regular">{words.checking}</Text>
        </View>
      ) : "failure" in outcome ? (
        <Notice title={words.failed} testID="wifi-diagnostics-failed" />
      ) : (
        <View style={{ gap: 16 }}>
          {outcome.found.link ? null : <Notice title={words.notJoined} testID="wifi-diagnostics-not-joined" />}
          <GroupedSection>
            {rows.map((row) => (
              <GroupedSectionItem
                key={row.key}
                title={row.title}
                value={row.value}
                description={row.detail || undefined}
                descriptionLines={0}
                testID={`wifi-diagnostics-${row.key}`}
              />
            ))}
          </GroupedSection>
        </View>
      )}
    </Dialog>
  );
}

/**
 * Wi-Fi (docs/adr/0038-networks.mdx, decision 7), as a Mac's: the switch; the network joined, its
 * signal and lock, and its sheet (Details…); the known networks, each with its menu (its sheet,
 * and forgetting it once confirmed); the other networks nearby, each joined when chosen; and, with
 * a network joined, Wi-Fi Diagnostics…. Settings never asks a password: the session asks it in its
 * own sheet, and says there why a join failed. The page asks for a scan when it opens and every
 * ten seconds while it is shown, as Control Center's view does.
 */
export function WifiPage({ snapshot, actions, strings, locale }: PageProps) {
  const words = strings.wifi;
  const state = snapshot.network;
  const act = actions.network;
  const scanning = Boolean(act && state?.available && state.wifi.device && state.wifi.enabled);
  const [sheet, sheetOpen, showSheet, hideSheet] = useHeld<string>();
  const [forgetting, forgetOpen, askForget, hideForget] = useHeld<{ known: KnownNetwork; fromSheet: boolean }>();
  const [note, noteOpen, showNote, hideNote] = useHeld<{ title: string; description: string }>();
  const [diagnosing, setDiagnosing] = React.useState(false);
  const [setting, setSetting] = React.useState<PrivateAddressChoice | null>(null);
  React.useEffect(() => {
    if (!scanning || !act) return;
    act.scan();
    const timer = setInterval(act.scan, SCAN_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [scanning, act]);
  // A network joined is said once, as it connects, as VoiceOver hears a Mac's Wi-Fi join: the
  // row it was joined from goes, so nothing focused would say it.
  const joinedName =
    state?.available && state.wifi.device && state.wifi.state === "connected" ? currentNetwork(state.wifi)?.ssid ?? "" : "";
  const saidJoined = React.useRef(joinedName);
  React.useEffect(() => {
    if (joinedName === saidJoined.current) return;
    saidJoined.current = joinedName;
    if (joinedName) AccessibilityInfo.announceForAccessibility(words.joined(joinedName));
  }, [joinedName]);
  const diagnose = React.useCallback(async () => {
    if (!act) return { failure: "unavailable" };
    return act.diagnose().then(
      (answer) => (answer && "wifiDiagnostics" in answer ? { found: answer.wifiDiagnostics } : { failure: "failed" }),
      () => ({ failure: "failed" }),
    );
  }, [act]);
  if (!state || !act || !state.available) return <Notice title={words.unavailable} testID="wifi-unavailable" />;
  const wifi = state.wifi;
  if (!wifi.device) return <Notice title={words.noDevice} testID="wifi-no-device" />;
  const current = currentNetwork(wifi);
  const currentName = current?.ssid ?? null;
  const currentState = wifi.state === "connecting" ? words.connecting : words.connected;
  const nearby = (ssid: string) => wifi.networks.find((network) => network.ssid === ssid);
  const others = otherNetworks(wifi.networks);
  const say = (shown: { title: string; description: string }) => {
    showNote(shown);
    // Said as it shows: the note's dialog does not take the screen reader's focus.
    AccessibilityInfo.announceForAccessibility(`${shown.title}. ${shown.description}`);
  };
  const join = async (ssid: string) => {
    const reason = await act.join(ssid).then(() => null, failureOf);
    if (joinFailureNeedsNote(reason)) {
      say({ title: words.couldNotJoin(ssid), description: failureLabel(reason, words.failures, "join") });
    }
  };
  const forget = async (known: KnownNetwork) => {
    const reason = await act.forget(known.id).then(() => null, failureOf);
    if (reason) showNote({ title: words.couldNotForget, description: failureLabel(reason, words.failures, "forget") });
  };
  const choose = async (known: KnownNetwork, mode: PrivateAddressChoice) => {
    setSetting(mode);
    const { failure, rejoinFailed } = await act
      .setPrivateAddress(known.id, mode)
      .then(privateAddressOutcome, privateAddressFailure);
    setSetting(null);
    if (failure) {
      const description = failure === "notPermitted" ? words.failures.notPermittedChange : words.failures[failure];
      say({ title: words.couldNotChange, description });
    } else if (rejoinFailed) {
      say({ title: words.couldNotJoin(known.ssid), description: words.couldNotRejoin(known.ssid) });
    }
  };
  return (
    <View style={PAGE}>
      <GroupedSection footer={wifi.enabled ? undefined : wifi.hardwareBlocked ? words.hardwareBlocked : words.off}>
        <GroupedSectionItem
          title={words.wifi}
          icon={<RiWifiLine />}
          testID="wifi"
          rightElement={
            <Switch
              size="xs"
              checked={wifi.enabled}
              onCheckedChange={act.setWifi}
              accessibilityLabel={words.wifi}
              testID="wifi-switch"
            />
          }
        />
        {wifi.enabled && currentName ? (
          <GroupedSectionItem
            title={currentName}
            description={currentState}
            leftInset={56}
            testID="wifi-current"
            accessibilityLabel={networkLabel(currentName, current ?? undefined, words, currentState)}
            rightElement={
              <View style={TRAILING}>
                <NetworkGlyphs network={current ?? undefined} />
                <Button
                  size="sm"
                  appearance="outline"
                  tone="neutral"
                  accessibilityLabel={words.detailsLabel(currentName)}
                  testID="wifi-details"
                  onPress={() => showSheet(currentName)}
                >
                  {words.details}
                </Button>
              </View>
            }
          />
        ) : null}
      </GroupedSection>
      {wifi.enabled && wifi.known.length > 0 ? (
        <GroupedSection title={words.known}>
          {wifi.known.map((known) => {
            // One nearby and not joined is joined by choosing it, as a Mac's list joins it; the
            // session has its password.
            const there = nearby(known.ssid);
            const joinable = !!there && !there.current;
            const joining = state.joining === known.ssid;
            return (
              <GroupedSectionItem
                key={known.id}
                title={known.ssid}
                testID={`wifi-known-${known.ssid}`}
                accessibilityLabel={networkLabel(known.ssid, there, words, joining ? words.connecting : null)}
                accessibilityState={{ busy: joining }}
                showChevron={false}
                onPress={
                  joinable
                    ? () => {
                        void join(known.ssid);
                      }
                    : undefined
                }
                rightElement={
                  <View style={TRAILING}>
                    <NetworkGlyphs network={nearby(known.ssid)} />
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          size="sm"
                          appearance="plain"
                          tone="neutral"
                          iconOnly
                          icon={RiMoreLine}
                          accessibilityLabel={words.more(known.ssid)}
                          testID={`wifi-known-${known.ssid}-more`}
                        />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent label={words.more(known.ssid)} align="end" size="sm">
                        <DropdownMenuItem testID={`wifi-settings-${known.ssid}`} onPress={() => showSheet(known.ssid)}>
                          {words.networkSettings}
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          testID={`wifi-forget-${known.ssid}`}
                          onPress={() => askForget({ known, fromSheet: false })}
                        >
                          {words.forgetItem}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </View>
                }
              />
            );
          })}
        </GroupedSection>
      ) : null}
      {wifi.enabled ? (
        <GroupedSection title={words.other}>
          {others.length === 0 ? (
            <GroupedSectionItem title={words.none} testID="wifi-none" />
          ) : (
            others.map((network) => {
              const joining = state.joining === network.ssid;
              const unsupported = network.supported ? undefined : words.notSupported;
              return (
                <GroupedSectionItem
                  key={network.ssid}
                  title={network.ssid}
                  description={unsupported}
                  showChevron={false}
                  testID={`wifi-network-${network.ssid}`}
                  accessibilityLabel={networkLabel(
                    network.ssid,
                    network,
                    words,
                    joining ? words.connecting : null,
                    unsupported,
                  )}
                  accessibilityState={{ busy: joining }}
                  onPress={() => {
                    void join(network.ssid);
                  }}
                  rightElement={
                    <View style={TRAILING}>
                      {joining ? (
                        <Loading
                          size="sm"
                          accessibilityLabel={words.joining(network.ssid)}
                          testID={`wifi-joining-${network.ssid}`}
                        />
                      ) : null}
                      <NetworkGlyphs network={network} />
                    </View>
                  }
                />
              );
            })
          )}
        </GroupedSection>
      ) : null}
      {wifi.enabled && wifi.state === "connected" ? (
        <View style={{ flexDirection: "row", justifyContent: "flex-end" }}>
          <Button
            size="sm"
            appearance="outline"
            tone="neutral"
            testID="wifi-diagnostics"
            onPress={() => setDiagnosing(true)}
          >
            {words.diagnostics.open}
          </Button>
        </View>
      ) : null}
      <NetworkSheet
        ssid={sheet ?? ""}
        open={sheetOpen}
        wifi={wifi}
        strings={strings}
        setting={setting}
        onClose={hideSheet}
        onChoose={(known, mode) => {
          void choose(known, mode);
        }}
        onForget={(known) => {
          hideSheet();
          askForget({ known, fromSheet: true });
        }}
      />
      <DiagnosticsSheet
        open={diagnosing}
        run={diagnose}
        strings={strings}
        locale={locale}
        onClose={() => setDiagnosing(false)}
      />
      <AlertDialog
        open={forgetOpen}
        onOpenChange={(open) => {
          if (!open) hideForget();
        }}
        title={words.forgetTitle(forgetting?.known.ssid ?? "")}
        description={words.forgetDescription}
        confirmLabel={words.forget}
        cancelLabel={words.cancel}
        destructive
        onConfirm={() => {
          if (forgetting) void forget(forgetting.known);
        }}
        // Cancelled from the network's sheet, the sheet comes back, as a Mac's alert returns to it.
        onCancel={() => {
          if (forgetting?.fromSheet) showSheet(forgetting.known.ssid);
        }}
        testID="wifi-forget-confirm"
      />
      <AlertDialog
        open={noteOpen}
        onOpenChange={(open) => {
          if (!open) hideNote();
        }}
        title={note?.title ?? ""}
        description={note?.description}
        confirmLabel={words.ok}
        hideCancel
        testID="wifi-failure"
      />
    </View>
  );
}

/** The names a reset lists. */
function listOf(names: string[]): string {
  return names.join(", ");
}

/**
 * Network (decision 7): Ethernet's state, speed and details; VPN only says it is not there yet,
 * since nothing in OxyOS configures one; and Reset Network Settings…, as an iPhone's: the session
 * is asked what would go, the confirmation lists it (the Wi-Fi networks and VPNs removed with
 * their passwords, and what is kept and why), and the session removes it, saying what came of it.
 */
export function NetworkPage({ snapshot, actions, strings, locale }: PageProps) {
  const words = strings.network;
  const language = messagesLanguage(locale);
  const state = snapshot.network;
  const act = actions.network;
  const [plan, planOpen, showPlan, hidePlan] = useHeld<ResetPlan>();
  const [note, noteOpen, showNote, hideNote] = useHeld<{ title: string; description: string }>();
  const [asking, setAsking] = React.useState(false);
  if (!state || !state.available) return <Notice title={words.unavailable} testID="network-unavailable" />;
  const ethernet = state.ethernet;
  const status = ethernetStatus(ethernet);
  const said = {
    none: words.noPort,
    connected: words.connected,
    unplugged: words.unplugged,
    connecting: words.connecting,
    notConnected: words.notConnected,
  }[status];
  const speed = status === "connected" ? formatSpeed(ethernet.speed, language) : "";
  const list = listOf;
  const say = (shown: { title: string; description: string }) => {
    showNote(shown);
    AccessibilityInfo.announceForAccessibility([shown.title, shown.description].filter(Boolean).join(". "));
  };
  const failed = (failure: string) =>
    say({
      title: words.couldNotReset,
      description: words.resetFailures[failure as keyof typeof words.resetFailures] ?? words.resetFailures.failed,
    });
  const preview = async () => {
    if (!act || asking) return;
    setAsking(true);
    const told = await act.resetPreview().then(resetPlanOf, resetFailureOf);
    setAsking(false);
    if ("plan" in told) showPlan(told.plan);
    else failed(told.failure);
  };
  const reset = async () => {
    if (!act) return;
    const told = await act.resetNetwork().then(resetPlanOf, resetFailureOf);
    if (!("plan" in told)) {
      failed(told.failure);
      return;
    }
    const refused = told.plan.kept.filter((kept) => kept.because === "failed").map((kept) => kept.name);
    say({ title: words.resetDone, description: refused.length ? words.resetKeptAfter(list(refused)) : "" });
  };
  const lines = plan ? resetLines(plan, words, list) : [];
  const nothing = !plan || plan.removed.length === 0;
  return (
    <View style={PAGE}>
      <GroupedSection title={words.ethernet}>
        <GroupedSectionItem
          title={words.ethernet}
          value={said}
          icon={<RiGlobalLine />}
          testID="ethernet-status"
          accessibilityLabel={`${words.ethernet}, ${said}`}
        />
        {speed ? <GroupedSectionItem title={words.speed} value={speed} testID="ethernet-speed" /> : null}
        {status === "connected"
          ? detailItems(ethernet.details, strings, strings.details.hardwareAddress, "ethernet")
          : null}
      </GroupedSection>
      <GroupedSection title={words.vpn}>
        <GroupedSectionItem title={words.vpnPlaceholder} titleNumberOfLines={0} testID="vpn-placeholder" />
      </GroupedSection>
      {act ? (
        <GroupedSection footer={words.resetFooter}>
          <GroupedSectionItem
            title={words.reset}
            destructive
            showChevron={false}
            accessibilityRole="button"
            accessibilityState={{ busy: asking }}
            onPress={() => {
              void preview();
            }}
            testID="network-reset"
          />
        </GroupedSection>
      ) : null}
      <AlertDialog
        open={planOpen}
        onOpenChange={(open) => {
          if (!open) hidePlan();
        }}
        title={words.resetTitle}
        description={lines.join("\n")}
        confirmLabel={nothing ? words.ok : words.resetConfirm}
        cancelLabel={words.cancel}
        hideCancel={nothing}
        destructive={!nothing}
        onConfirm={() => {
          if (!nothing) void reset();
        }}
        testID="network-reset-confirm"
      />
      <AlertDialog
        open={noteOpen}
        onOpenChange={(open) => {
          if (!open) hideNote();
        }}
        title={note?.title ?? ""}
        description={note?.description || undefined}
        confirmLabel={words.ok}
        hideCancel
        testID="network-reset-note"
      />
    </View>
  );
}
