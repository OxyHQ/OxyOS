/**
 * OxyOS's Settings (apps/settings) in the Bloom host (docs/adr/0019-settings.mdx). Its model is the
 * session's: every preference as the session keeps it (`OxyOSSettings`, which only the client the
 * session granted `settings` has), a change asked of the session, which validates, keeps and
 * applies it and tells every application; the displays as the session shows them and their changes
 * (docs/adr/0018-several-displays.mdx); the machine's facts; and the networks as the session
 * tells them (docs/adr/0038-networks.mdx). Without the grant (a host run outside a session, the
 * browser preview) the model is a fixture kept in memory.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Settings } from '../../../apps/settings/src/Settings';
import { stringsFor } from '../../../apps/settings/src/strings';
import {
  available,
  network as systemNetwork,
  privacy,
  updates as systemUpdates,
  useNetworkState,
  useUpdateState,
} from './oxyos/system';
import type { PrivacyApplication, UpdatesActions } from '../../../apps/settings/src/updates';
import type { NetworkActions, NetworkAnswer, NetworkState } from '../../../apps/settings/src/network';
import {
  PREFERENCE_KEYS,
  preferenceText,
  type Appearance,
  type DisplaysSnapshot,
  type NotificationPreviews,
  type Preferences,
  type SettingsActions,
  type SettingsSnapshot,
  type SystemFacts,
} from '../../../apps/settings/src/model';

const native = TurboModuleRegistry.get<{
  getValues(): Record<string, string> | null;
  setPreference(key: string, value: string): number;
  getSystem(): SystemFacts;
  getDisplays(): DisplaysSnapshot | null;
  setDisplayMode(output: number, width: number, height: number, refreshMhz: number): number;
  setDisplayScale(output: number, scale: number): number;
  setDisplayOrigin(output: number, x: number, y: number): number;
  setPrimaryDisplay(output: number): number;
  confirmDisplays(keep: boolean): number;
}>('OxyOSSettings');

const FIXTURE: SettingsSnapshot = {
  preferences: {
    appearance: 'auto',
    accent: 'oxy',
    reduceMotion: false,
    reduceTransparency: false,
    wallpaper: 'solar-gradients',
    lockAfter: 5,
    screenReader: false,
    clock24h: true,
    doNotDisturb: false,
    notificationPreviews: 'unlocked',
  },
  wallpapers: [{ id: 'solar-gradients', name: 'Solar Gradients' }],
  system: {
    computerName: 'ThinkPad P14s',
    version: 'OxyOS (development)',
    processor: '12th Gen Intel Core i7-1270P',
    memoryBytes: 32 * 1024 ** 3,
    display: '1920 × 1200',
    timeZone: 'Europe/Madrid',
    automaticTime: true,
    keyboardLayout: 'us',
  },
  // The ThinkPad's panel at 1.25, and a 27-inch 4K monitor right of it at 1.5.
  displays: {
    displays: [
      {
        output: 1,
        name: 'eDP-1',
        builtin: true,
        primary: true,
        x: 0,
        y: 0,
        width: 1536,
        height: 960,
        scale: 150,
        mode: 0,
        modes: [
          { width: 1920, height: 1200, refreshMhz: 60000, preferred: true },
          { width: 1280, height: 800, refreshMhz: 60000, preferred: false },
        ],
        scales: [120, 150, 180, 210],
        recommended: 150,
        widthMm: 301,
        heightMm: 188,
      },
      {
        output: 2,
        name: 'DELL U2723QE',
        builtin: false,
        primary: false,
        x: 1536,
        y: 0,
        width: 2560,
        height: 1440,
        scale: 180,
        mode: 0,
        modes: [
          { width: 3840, height: 2160, refreshMhz: 60000, preferred: true },
          { width: 2560, height: 1440, refreshMhz: 59951, preferred: false },
          { width: 1920, height: 1080, refreshMhz: 60000, preferred: false },
        ],
        scales: [120, 150, 180, 210, 240],
        recommended: 180,
        widthMm: 597,
        heightMm: 336,
      },
    ],
    trial: null,
  },
};

/** The reference ThinkPad's networks (ADR 0038): joined to LukeSkyRouter, a few others nearby (one
    open, one enterprise network the session cannot join yet), the cable unplugged. */
const NETWORK_FIXTURE: NetworkState = {
  available: true,
  wifi: {
    device: true,
    enabled: true,
    hardwareBlocked: false,
    state: 'connected',
    current: 'LukeSkyRouter',
    networks: [
      { ssid: 'LukeSkyRouter', signal: 73, security: 'wpa', known: true, current: true, supported: true },
      { ssid: 'eduroam', signal: 64, security: 'enterprise', known: false, current: false, supported: false },
      { ssid: 'Café Libre', signal: 58, security: 'open', known: false, current: false, supported: true },
      { ssid: 'Vecinos_5G', signal: 41, security: 'wpa3', known: false, current: false, supported: true },
      { ssid: 'Oficina', signal: 30, security: 'wpa', known: true, current: false, supported: true },
    ],
    known: [
      { id: '6f1c2a4e-0b8d-4f3e-9a51-2d7c8e1b9f01', ssid: 'LukeSkyRouter', autoJoin: true },
      { id: 'b2e94d17-5c3a-4a8f-8e6d-0f1a2b3c4d02', ssid: 'Oficina', autoJoin: true },
      { id: 'd8a7c6b5-4e3f-4a2b-9c1d-0e9f8a7b6c03', ssid: 'Casa Abuela', autoJoin: true },
    ],
    details: {
      interface: 'wlp0s20f3',
      hardwareAddress: '3c:58:c2:1a:7e:90',
      ipv4: { address: '192.168.1.42', prefix: 24, subnetMask: '255.255.255.0' },
      router: '192.168.1.1',
      dns: ['192.168.1.1'],
      ipv6: ['fe80::3e58:c2ff:fe1a:7e90'],
    },
  },
  ethernet: { device: true, state: 'unplugged', speed: 0, details: null },
  joining: null,
};

/** The session's preferences, as its file keeps them, read into the application's model. */
function preferencesOf(values: Record<string, string>): Preferences {
  const flag = (key: string) => values[key] === 'true';
  return {
    appearance: (values[PREFERENCE_KEYS.appearance] as Appearance) ?? 'auto',
    accent: values[PREFERENCE_KEYS.accent] ?? 'oxy',
    reduceMotion: flag(PREFERENCE_KEYS.reduceMotion),
    reduceTransparency: flag(PREFERENCE_KEYS.reduceTransparency),
    wallpaper: values[PREFERENCE_KEYS.wallpaper] || null,
    lockAfter: Number(values[PREFERENCE_KEYS.lockAfter] ?? 5),
    screenReader: flag(PREFERENCE_KEYS.screenReader),
    clock24h: flag(PREFERENCE_KEYS.clock24h),
    doNotDisturb: flag(PREFERENCE_KEYS.doNotDisturb),
    notificationPreviews:
      (values[PREFERENCE_KEYS.notificationPreviews] as NotificationPreviews) ?? 'unlocked',
  };
}

/** The session's model when this is Settings in a session, else the fixture's. */
function useSettingsModel(): { snapshot: SettingsSnapshot; actions: SettingsActions } {
  const [values, setValues] = React.useState(() => native?.getValues() ?? null);
  const [displays, setDisplays] = React.useState(() => native?.getDisplays() ?? null);
  const [fixture, setFixture] = React.useState(FIXTURE);
  React.useEffect(() => {
    const subscriptions = [
      DeviceEventEmitter.addListener('OxyOSSettingsChanged', setValues),
      DeviceEventEmitter.addListener('OxyOSDisplaysChanged', setDisplays),
    ];
    // A change between the first read and the subscription is not lost.
    setValues(native?.getValues() ?? null);
    setDisplays(native?.getDisplays() ?? null);
    return () => subscriptions.forEach((subscription) => subscription.remove());
  }, []);
  const system = React.useMemo(() => (native ? native.getSystem() : FIXTURE.system), []);
  const actions = React.useMemo<SettingsActions>(
    () => ({
      setPreference: (key, value) => {
        // The session answers with the preference as it now keeps it (OxyOSSettingsChanged); a
        // refused change leaves it as it was.
        if (native?.getValues()) native.setPreference(PREFERENCE_KEYS[key], preferenceText(value));
        else setFixture((current) => ({ ...current, preferences: { ...current.preferences, [key]: value } }));
      },
      // The session judges, applies and keeps each and tells the displays as they are then
      // (OxyOSDisplaysChanged); the fixture takes a scale and the main display as they are asked.
      setDisplayMode: (output, mode) => {
        if (native?.getValues()) native.setDisplayMode(output, mode.width, mode.height, mode.refreshMhz);
      },
      setDisplayScale: (output, scale) => {
        if (native?.getValues()) native.setDisplayScale(output, scale);
        else
          setFixture((current) => ({
            ...current,
            displays: {
              ...current.displays,
              displays: current.displays.displays.map((d) => (d.output === output ? { ...d, scale } : d)),
            },
          }));
      },
      setDisplayOrigin: (output, x, y) => {
        if (native?.getValues()) native.setDisplayOrigin(output, x, y);
      },
      setPrimaryDisplay: (output) => {
        if (native?.getValues()) native.setPrimaryDisplay(output);
        else
          setFixture((current) => ({
            ...current,
            displays: {
              ...current.displays,
              displays: current.displays.displays.map((d) => ({ ...d, primary: d.output === output })),
            },
          }));
      },
      confirmDisplays: (keep) => {
        if (native?.getValues()) native.confirmDisplays(keep);
      },
    }),
    [],
  );
  if (!values) return { snapshot: fixture, actions };
  return {
    snapshot: {
      preferences: preferencesOf(values),
      // The wallpapers the session has: its own, for now.
      wallpapers: FIXTURE.wallpapers,
      system,
      displays: displays ?? { displays: [], trial: null },
    },
    actions,
  };
}

/**
 * Software Update and Privacy (docs/adr/0030-installing-applications.mdx, decisions 8 and 16): the
 * install service's one update state, followed, and the account's applications' capabilities, as
 * the session tells Settings with its `updates` and `privacy` grants. The OS's own row says this
 * install updates with apt: no OxyOS install updates otherwise until ADR 0023's image updates exist.
 */
function useInstallModel() {
  const state = useUpdateState();
  const [failed, setFailed] = React.useState<string | null>(null);
  // Without the `updates` grant the session refuses: the page says updates are not available here.
  const [updatesAllowed, setUpdatesAllowed] = React.useState(available);
  React.useEffect(() => {
    if (available) systemUpdates.state().then((answer: any) => answer?.refused && setUpdatesAllowed(false));
  }, []);
  const [apps, setApps] = React.useState<PrivacyApplication[] | null | undefined>(available ? null : undefined);
  const reload = React.useCallback(async () => {
    const answer = await privacy.list();
    if (answer && Array.isArray(answer.privacy)) setApps(answer.privacy);
    else if (answer && answer.refused) setApps(undefined);
  }, []);
  React.useEffect(() => {
    if (available) reload();
  }, [reload]);
  React.useEffect(() => {
    if (available && state) reload();
  }, [state, reload]);
  const done = (answer: { refused?: { reason: string }; failed?: { reason: string } } | null) =>
    setFailed(answer?.refused?.reason ?? answer?.failed?.reason ?? null);
  const updates: UpdatesActions = {
    checkForUpdates: () => systemUpdates.check().then(done),
    update: (id) => systemUpdates.update([id]).then(done),
    updateAll: () => systemUpdates.updateAll().then(done),
  };
  return {
    updates: updatesAllowed ? { system: { kind: 'apt' as const }, state, failed } : undefined,
    updatesActions: updatesAllowed ? updates : undefined,
    privacy: apps,
    privacyActions: available
      ? { decide: (id: string, capability: string, granted: boolean) => privacy.decide(id, capability, granted).then(reload) }
      : undefined,
  };
}

/**
 * Wi-Fi and networks (docs/adr/0038-networks.mdx): the state the session tells a client with its
 * `network` grant, followed, and its requests. The session tells nothing without the grant, so the
 * pages are there only once it has told the state. Without a session the fixture joins a supported
 * network after a moment (no password asked: that is the session's sheet) and forgets a known one.
 */
function useNetworkModel(): { network: NetworkState | undefined; actions: NetworkActions | undefined } {
  const told: NetworkState | null = useNetworkState();
  const [fixture, setFixture] = React.useState(NETWORK_FIXTURE);
  const current = React.useRef(fixture);
  current.current = fixture;
  const session = React.useMemo<NetworkActions>(
    () => ({
      setWifi: (enabled) => {
        systemNetwork.setWifi(enabled);
      },
      join: (ssid) => systemNetwork.join(ssid) as Promise<NetworkAnswer>,
      forget: (id) => systemNetwork.forget(id) as Promise<NetworkAnswer>,
      scan: () => {
        systemNetwork.scan();
      },
    }),
    [],
  );
  const local = React.useMemo<NetworkActions>(
    () => ({
      setWifi: (enabled) =>
        setFixture((state) => ({
          ...state,
          // On, it sees the fixture's networks again and joins none until one is chosen.
          wifi: enabled
            ? {
                ...state.wifi,
                enabled: true,
                state: 'disconnected',
                networks: NETWORK_FIXTURE.wifi.networks.map((n) => ({
                  ...n,
                  current: false,
                  known: state.wifi.known.some((k) => k.ssid === n.ssid),
                })),
              }
            : { ...state.wifi, enabled: false, state: 'off', current: null, networks: [], details: null },
        })),
      join: async (ssid) => {
        const target = current.current.wifi.networks.find((n) => n.ssid === ssid);
        if (!target) return { failed: { reason: 'notFound' } };
        if (!target.supported) return { failed: { reason: 'unsupported' } };
        setFixture((state) => ({ ...state, joining: ssid }));
        await new Promise((resolve) => setTimeout(resolve, 1500));
        setFixture((state) => ({
          ...state,
          joining: null,
          wifi: {
            ...state.wifi,
            state: 'connected',
            current: ssid,
            details: NETWORK_FIXTURE.wifi.details,
            networks: state.wifi.networks.map((n) => ({
              ...n,
              current: n.ssid === ssid,
              known: n.known || n.ssid === ssid,
            })),
            known: state.wifi.known.some((k) => k.ssid === ssid)
              ? state.wifi.known
              : [...state.wifi.known, { id: `fixture-${ssid}`, ssid, autoJoin: true }],
          },
        }));
        return { joined: { ssid } };
      },
      forget: async (id) => {
        const known = current.current.wifi.known.find((k) => k.id === id);
        if (!known) return { failed: { reason: 'notFound' } };
        setFixture((state) => {
          const joined = state.wifi.current === known.ssid;
          return {
            ...state,
            wifi: {
              ...state.wifi,
              state: joined ? 'disconnected' : state.wifi.state,
              current: joined ? null : state.wifi.current,
              details: joined ? null : state.wifi.details,
              networks: state.wifi.networks.map((n) =>
                n.ssid === known.ssid ? { ...n, known: false, current: false } : n,
              ),
              known: state.wifi.known.filter((k) => k.id !== id),
            },
          };
        });
        return { forgotten: {} };
      },
      scan: () => {},
    }),
    [],
  );
  if (!available) return { network: fixture, actions: local };
  return told ? { network: told, actions: session } : { network: undefined, actions: undefined };
}

export default function SettingsApp() {
  const model = useSettingsModel();
  const install = useInstallModel();
  const networks = useNetworkModel();
  const snapshot = { ...model.snapshot, updates: install.updates, privacy: install.privacy, network: networks.network };
  const actions = {
    ...model.actions,
    updates: install.updatesActions,
    privacy: install.privacyActions,
    network: networks.actions,
  };
  const strings = React.useMemo(() => stringsFor(Intl.DateTimeFormat().resolvedOptions().locale), []);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces}>
        <Settings snapshot={snapshot} actions={actions} strings={strings} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
