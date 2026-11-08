/**
 * OxyOS's Settings (apps/settings) in the Bloom host (docs/adr/0019-settings.mdx). Its model is the
 * session's: every preference as the session keeps it (`OxyOSSettings`, which only the client the
 * session granted `settings` has), a change asked of the session, which validates, keeps and
 * applies it and tells every application; the displays as the session shows them and their changes
 * (docs/adr/0018-several-displays.mdx); the machine's facts; and the networks and Bluetooth as the
 * session tells them (docs/adr/0038-networks.mdx, docs/adr/0055-bluetooth.mdx). Without the grant
 * (a host run outside a session, the browser preview) the model is a fixture kept in memory. Its
 * language is the session's locale (docs/adr/0040-language-and-region.mdx, decision 5), given to
 * `BloomProvider` and followed live;
 * the fixture's is the one its own preferences give. Its Oxy account is the machine's Oxy session
 * as the platform gives every application (oxyos/session-host.js, docs/adr/0020-oxy-account-login.mdx):
 * signing out is the session's (`signOut`, with the `settings` grant), removing the account the
 * accounts' service's, which only Settings may ask, as is erasing the computer (General › Transfer
 * or Reset, docs/adr/0054-recovery-and-erasing.mdx).
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { Directory, File, Paths } from 'expo-file-system';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Settings } from '../../../apps/settings/src/Settings';
import { sessionLocale } from '../../../apps/settings/src/locale';
import { messagesLanguage } from '../../../apps/settings/src/messages';
import {
  addedText,
  EMPTY_CATALOG,
  parseAdded,
  uriPath,
  type AddedWallpaper,
  type WallpaperCatalog,
} from '../../../apps/settings/src/wallpaper';
import { useSessionLocale } from './oxyos/locale';
import {
  available,
  session as systemSession,
  bluetooth as systemBluetooth,
  network as systemNetwork,
  power as systemPower,
  privacy,
  sound as systemSound,
  updates as systemUpdates,
  followInputLevel,
  useInputSourcesState,
  useBluetoothState,
  useNetworkState,
  useSoundState,
  usePowerState,
  useUpdateState,
} from './oxyos/system';
import type { PrivacyApplication, UpdateOutcome, UpdatesActions } from '../../../apps/settings/src/updates';
import {
  accountSnapshotFrom,
  type AccountActions,
  type AccountOutcome,
  type AccountSnapshot,
} from '../../../apps/settings/src/account';
import { eraseOutcomeFrom, erasePreviewFrom, type EraseActions } from '../../../apps/settings/src/erase';
import { localOutcomeFrom, localSignInFrom } from '../../../apps/settings/src/local-sign-in';
import type { NetworkActions, NetworkAnswer, NetworkState } from '../../../apps/settings/src/network';
import type {
  BluetoothActions,
  BluetoothAnswer,
  BluetoothDevice,
  BluetoothState,
} from '../../../apps/settings/src/bluetooth';
import type { SoundActions, SoundDirection, SoundSide, SoundState } from '../../../apps/settings/src/sound';
import {
  chargeLimitFailureOf,
  energyOf,
  historyOf,
  type AppEnergy,
  type HistoryChart,
  type LowPowerMode,
  type PowerActions,
  type PowerHistory,
  type PowerState,
} from '../../../apps/settings/src/battery';
import {
  INPUT_METHODS,
  inputSourcesOf,
  parseInputSources,
  parseInputSourcesPreference,
} from '../../../apps/settings/src/input-sources';
import {
  ENERGY_DEFAULTS,
  GESTURE_DEFAULTS,
  PREFERENCE_KEYS,
  preferenceText,
  swipeFingersOf,
  TRIAL_SECONDS,
  type Appearance,
  type DisplaysSnapshot,
  type HourCycle,
  type LanguagePreference,
  type NotificationPreviews,
  type Preferences,
  type SettingsActions,
  type SettingsSnapshot,
  type SystemFacts,
  type WallpaperActions,
} from '../../../apps/settings/src/model';

const native = TurboModuleRegistry.get<{
  getValues(): Record<string, string> | null;
  setPreference(key: string, value: string): number;
  getSystem(): SystemFacts;
  getApplications?(): { id: string; name: string }[];
  getDisplays(): DisplaysSnapshot | null;
  setDisplayMode(output: number, width: number, height: number, refreshMhz: number): number;
  setDisplayScale(output: number, scale: number): number;
  setDisplayOrigin(output: number, x: number, y: number): number;
  setPrimaryDisplay(output: number): number;
  confirmDisplays(keep: boolean): number;
  // Settings › Wallpaper (docs/adr/0048-wallpapers.mdx, decision 7): the system's catalog in a
  // language, a cached thumbnail and a folder's images, each a promise.
  getWallpapers?(language: string): WallpaperCatalog | null;
  wallpaperThumbnail?(uri: string, width: number, height: number): Promise<string | null>;
  wallpaperFolder?(path: string): Promise<string[]>;
}>('OxyOSSettings');

const FIXTURE: SettingsSnapshot = {
  preferences: {
    appearance: 'auto',
    accent: 'oxy',
    reduceMotion: false,
    reduceTransparency: false,
    caretBlink: true,
    uiSounds: true,
    trackpadNaturalScrolling: true,
    mouseNaturalScrolling: false,
    ...GESTURE_DEFAULTS,
    wallpaper: '',
    wallpaperAllDesktops: true,
    lockAfter: 5,
    screenReader: false,
    language: '',
    region: '',
    hourCycle: 'auto',
    doNotDisturb: false,
    notificationPreviews: 'unlocked',
    notificationSoundsOff: [],
    openAtLogin: [],
    inputSources: [],
    inputSource: 'layout',
    ...ENERGY_DEFAULTS,
  },
  wallpapers: EMPTY_CATALOG,
  applications: [
    { id: 'so.oxy.allo', name: 'Allo' },
    { id: 'so.oxy.inbox', name: 'Inbox' },
  ],
  system: {
    computerName: 'ThinkPad P14s',
    version: 'OxyOS (development)',
    processor: '12th Gen Intel Core i7-1270P',
    memoryBytes: 32 * 1024 ** 3,
    display: '1920 × 1200',
    timeZone: 'Europe/Madrid',
    automaticTime: true,
    keyboardLayout: 'us',
    // The runtime's, which is the environment's, as a host outside a session has it.
    locale: Intl.DateTimeFormat().resolvedOptions().locale,
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
    current: { ssid: 'LukeSkyRouter', signal: 73, security: 'wpa', known: true, current: true, supported: true },
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

/** The reference ThinkPad's Bluetooth (ADR 0055): on, its paired AirPods and mouse connected, its
    keyboard not; a speaker and a phone nearby, seen once the page discovers. */
const BLUETOOTH_PAIRED: BluetoothDevice[] = [
  {
    id: '7C:04:D0:A1:3E:52', address: '7C:04:D0:A1:3E:52', name: 'AirPods Pro', kind: 'headphones',
    paired: true, connected: true, busy: null, battery: 80, audio: true, input: false,
  },
  {
    id: 'F0:B3:EC:21:6A:9D', address: 'F0:B3:EC:21:6A:9D', name: 'Magic Keyboard', kind: 'keyboard',
    paired: true, connected: false, busy: null, battery: 64, audio: false, input: true,
  },
  {
    id: 'D4:3B:04:7F:12:C8', address: 'D4:3B:04:7F:12:C8', name: 'MX Master 3S', kind: 'mouse',
    paired: true, connected: true, busy: null, battery: 55, audio: false, input: true,
  },
];
const BLUETOOTH_NEARBY: BluetoothDevice[] = [
  {
    id: '00:42:79:B6:58:E1', address: '00:42:79:B6:58:E1', name: 'JBL Flip 6', kind: 'speaker',
    paired: false, connected: false, busy: null, battery: null, audio: true, input: false,
  },
  {
    id: '58:24:29:9C:0D:3F', address: '58:24:29:9C:0D:3F', name: 'Pixel 8', kind: 'phone',
    paired: false, connected: false, busy: null, battery: null, audio: false, input: false,
  },
];
const BLUETOOTH_FIXTURE: BluetoothState = {
  available: true,
  adapter: {
    name: 'ThinkPad',
    address: '3C:58:C2:1A:7E:91',
    powered: true,
    blocked: false,
    discovering: false,
    discoverable: false,
  },
  devices: BLUETOOTH_PAIRED,
};
/** Each fixture device's own name, which an empty rename gives back. */
const BLUETOOTH_NAMES = new Map(
  [...BLUETOOTH_PAIRED, ...BLUETOOTH_NEARBY].map((device) => [device.id, device.name]),
);

/** The session's preferences, as its file keeps them, read into the application's model. */
function preferencesOf(values: Record<string, string>): Preferences {
  const flag = (key: string) => values[key] === 'true';
  return {
    appearance: (values[PREFERENCE_KEYS.appearance] as Appearance) ?? 'auto',
    accent: values[PREFERENCE_KEYS.accent] ?? 'oxy',
    reduceMotion: flag(PREFERENCE_KEYS.reduceMotion),
    reduceTransparency: flag(PREFERENCE_KEYS.reduceTransparency),
    // On unless the session keeps it off (crates/oxyos-preferences).
    caretBlink: values[PREFERENCE_KEYS.caretBlink] !== 'false',
    // On unless the session keeps it off (docs/adr/0044-system-sounds.mdx).
    uiSounds: values[PREFERENCE_KEYS.uiSounds] !== 'false',
    // A trackpad's on and a mouse's off unless the session keeps otherwise
    // (docs/adr/0042-scrolling.mdx, decision 6).
    trackpadNaturalScrolling: values[PREFERENCE_KEYS.trackpadNaturalScrolling] !== 'false',
    mouseNaturalScrolling: flag(PREFERENCE_KEYS.mouseNaturalScrolling),
    // Trackpad's More Gestures (docs/adr/0046-interactive-gestures.mdx): three fingers for each swipe
    // unless the session keeps four or off, and Launchpad's pinch on unless it keeps it off.
    missionControlSwipe: swipeFingersOf(values[PREFERENCE_KEYS.missionControlSwipe]),
    appExposeSwipe: swipeFingersOf(values[PREFERENCE_KEYS.appExposeSwipe]),
    desktopsSwipe: swipeFingersOf(values[PREFERENCE_KEYS.desktopsSwipe]),
    launchpadPinch: values[PREFERENCE_KEYS.launchpadPinch] !== 'false',
    // The choice where Settings' window is (docs/adr/0048-wallpapers.mdx, decision 4).
    wallpaper: values[PREFERENCE_KEYS.wallpaper] ?? '',
    // On unless the session keeps it off.
    wallpaperAllDesktops: values[PREFERENCE_KEYS.wallpaperAllDesktops] !== 'false',
    lockAfter: Number(values[PREFERENCE_KEYS.lockAfter] ?? 5),
    screenReader: flag(PREFERENCE_KEYS.screenReader),
    // The system's unless the session keeps the person's (crates/oxyos-preferences).
    language: (values[PREFERENCE_KEYS.language] as LanguagePreference) ?? '',
    region: values[PREFERENCE_KEYS.region] ?? '',
    hourCycle: (values[PREFERENCE_KEYS.hourCycle] as HourCycle) || 'auto',
    doNotDisturb: flag(PREFERENCE_KEYS.doNotDisturb),
    notificationPreviews:
      (values[PREFERENCE_KEYS.notificationPreviews] as NotificationPreviews) ?? 'unlocked',
    // The applications whose notifications are silent, as the session tells Settings them.
    notificationSoundsOff: (values[PREFERENCE_KEYS.notificationSoundsOff] ?? '').split(',').filter(Boolean),
    // The applications opened at login, as the session tells Settings them (the dock's list too,
    // docs/adr/0060-login-items-and-background-activity.mdx).
    openAtLogin: (values[PREFERENCE_KEYS.openAtLogin] ?? '').split(',').filter(Boolean),
    // None added, the layout current, unless the session keeps others (docs/adr/0041-input-methods.mdx).
    inputSources: parseInputSourcesPreference(values[PREFERENCE_KEYS.inputSources]),
    inputSource: values[PREFERENCE_KEYS.inputSource] || 'layout',
    // Battery and energy (docs/adr/0057-battery-and-energy.mdx): the session's defaults unless it
    // keeps others (crates/oxyos-preferences).
    batteryPercentage: flag(PREFERENCE_KEYS.batteryPercentage),
    lowPowerMode: (values[PREFERENCE_KEYS.lowPowerMode] as LowPowerMode) || ENERGY_DEFAULTS.lowPowerMode,
    displayOffBattery: minutesOf(values[PREFERENCE_KEYS.displayOffBattery], ENERGY_DEFAULTS.displayOffBattery),
    displayOffAdapter: minutesOf(values[PREFERENCE_KEYS.displayOffAdapter], ENERGY_DEFAULTS.displayOffAdapter),
    sleepBattery: minutesOf(values[PREFERENCE_KEYS.sleepBattery], ENERGY_DEFAULTS.sleepBattery),
    sleepAdapter: minutesOf(values[PREFERENCE_KEYS.sleepAdapter], ENERGY_DEFAULTS.sleepAdapter),
    dimOnBattery: values[PREFERENCE_KEYS.dimOnBattery] !== 'false',
  };
}

/** A timer's minutes as the session's file keeps them, or its default. */
function minutesOf(text: string | undefined, fallback: number): number {
  const minutes = Number(text);
  return text !== undefined && text !== '' && Number.isInteger(minutes) && minutes >= 0 ? minutes : fallback;
}

/** The session's model when this is Settings in a session, else the fixture's. */
function useSettingsModel(): { snapshot: SettingsSnapshot; actions: SettingsActions; session: boolean } {
  const [values, setValues] = React.useState(() => native?.getValues() ?? null);
  const [displays, setDisplays] = React.useState(() => native?.getDisplays() ?? null);
  // The displays as they were before a mode went on trial, which a revert brings back.
  const shown = React.useRef(displays);
  shown.current = displays;
  const beforeTrial = React.useRef<typeof displays>(null);
  // The mode asked for, until the session answers it: refused, the trial shown for it goes.
  const askedMode = React.useRef(0);
  const [fixture, setFixture] = React.useState(FIXTURE);
  React.useEffect(() => {
    const subscriptions = [
      DeviceEventEmitter.addListener('OxyOSSettingsChanged', setValues),
      DeviceEventEmitter.addListener('OxyOSDisplaysChanged', setDisplays),
      DeviceEventEmitter.addListener('OxyOSSettingsResult', ({ serial, accepted }) => {
        if (serial !== askedMode.current) return;
        askedMode.current = 0;
        const before = beforeTrial.current;
        if (!accepted && before) {
          beforeTrial.current = null;
          setDisplays({ ...before, trial: null });
        }
      }),
    ];
    // A change between the first read and the subscription is not lost.
    setValues(native?.getValues() ?? null);
    setDisplays(native?.getDisplays() ?? null);
    return () => subscriptions.forEach((subscription) => subscription.remove());
  }, []);
  // The session's applications as its registry names them, read again as the host says they changed
  // (one installed, updated or removed while Settings is open, or the session's language changed).
  const [applications, setApplications] = React.useState(() => native?.getApplications?.() ?? []);
  React.useEffect(() => {
    const read = () => setApplications(native?.getApplications?.() ?? []);
    const subscription = DeviceEventEmitter.addListener('OxyOSApplicationsChanged', read);
    return () => subscription.remove();
  }, []);
  const system = React.useMemo(() => {
    if (!native) return FIXTURE.system;
    const told = native.getSystem();
    // A host that says nothing of the system's locale: the runtime's, which is its environment's.
    return { ...told, locale: told.locale || Intl.DateTimeFormat().resolvedOptions().locale };
  }, []);
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
      // A mode asked for is on trial (docs/adr/0018-several-displays.mdx: for TRIAL_SECONDS, unless
      // the card refuses it): shown so at once, as the session will tell it, so the dialog that asks
      // to keep it comes as the choices close rather than after the session's answer, whose time
      // no one can tell.
      setDisplayMode: (output, mode) => {
        if (!native?.getValues()) return;
        const current = shown.current;
        if (!current?.trial) beforeTrial.current = current;
        askedMode.current = native.setDisplayMode(output, mode.width, mode.height, mode.refreshMhz);
        const display = current?.displays.find((d) => d.output === output);
        const shownMode = display?.modes[display.mode];
        const already =
          shownMode?.width === mode.width && shownMode.height === mode.height && shownMode.refreshMhz === mode.refreshMhz;
        if (current && !current.trial && !already) setDisplays({ ...current, trial: { output, seconds: TRIAL_SECONDS } });
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
      // The answer is shown at once, as the session will tell it: the trial over, the displays as
      // they were before it unless kept. The dialog that asked goes in the same commit, so the
      // control it gives the keyboard back to already shows (and says) the mode kept.
      confirmDisplays: (keep) => {
        if (!native?.getValues()) return;
        native.confirmDisplays(keep);
        const before = beforeTrial.current;
        beforeTrial.current = null;
        setDisplays((current: typeof displays) =>
          current?.trial ? (keep || !before ? { ...current, trial: null } : { ...before, trial: null }) : current,
        );
      },
    }),
    [],
  );
  if (!values) return { snapshot: fixture, actions, session: false };
  return {
    session: true,
    snapshot: {
      preferences: preferencesOf(values),
      // The system's catalog, which SettingsApp reads in the session's language.
      wallpapers: EMPTY_CATALOG,
      // The session's applications, for Notifications' sound of each (docs/adr/0044-system-sounds.mdx).
      applications,
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
  // Told again, unasked, when it changed elsewhere: an application granted a capability as it
  // registered, or one running in the background or no longer
  // (docs/adr/0060-login-items-and-background-activity.mdx).
  React.useEffect(() => (available ? privacy.subscribe(setApps) : undefined), []);
  const done = (answer: { refused?: { reason: string }; failed?: { reason: string } } | null) =>
    setFailed(answer?.refused?.reason ?? answer?.failed?.reason ?? null);
  // An update answers once it ended, with each application's outcome.
  const outcomes = (answer: any): UpdateOutcome[] => {
    done(answer);
    return Array.isArray(answer?.updated) ? answer.updated : [];
  };
  const updates: UpdatesActions = {
    checkForUpdates: () => systemUpdates.check().then(done),
    update: (id) => systemUpdates.update([id]).then(outcomes),
    updateAll: () => systemUpdates.updateAll().then(outcomes),
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
        setFixture((state) => {
          const networks = state.wifi.networks.map((n) => ({
            ...n,
            current: n.ssid === ssid,
            known: n.known || n.ssid === ssid,
          }));
          return {
          ...state,
          joining: null,
          wifi: {
            ...state.wifi,
            state: 'connected',
            current: networks.find((n) => n.ssid === ssid) ?? null,
            details: NETWORK_FIXTURE.wifi.details,
            networks,
            known: state.wifi.known.some((k) => k.ssid === ssid)
              ? state.wifi.known
              : [...state.wifi.known, { id: `fixture-${ssid}`, ssid, autoJoin: true }],
          },
          };
        });
        return { joined: { ssid } };
      },
      forget: async (id) => {
        const known = current.current.wifi.known.find((k) => k.id === id);
        if (!known) return { failed: { reason: 'notFound' } };
        setFixture((state) => {
          const joined = state.wifi.current?.ssid === known.ssid;
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

/** A machine's sound without a session: the built-in speakers chosen and a display's beside them,
    the built-in microphone chosen. */
const SOUND_FIXTURE: SoundState = {
  available: true,
  output: {
    devices: [
      { id: 'speakers', name: 'Speakers', kind: 'speakers', transport: 'builtIn', selected: true },
      { id: 'dell', name: 'DELL U2720Q', kind: 'hdmi', transport: 'hdmi', selected: false },
    ],
    volume: 0.5,
    muted: false,
    balance: 0,
  },
  input: {
    devices: [{ id: 'microphone', name: 'Microphone', kind: 'microphone', transport: 'builtIn', selected: true }],
    volume: 0.75,
    muted: false,
    balance: null,
  },
};

/**
 * Sound's outputs and inputs (docs/adr/0056-sound-devices.mdx): in a session, the state it tells
 * Settings (its `settings` grant), followed, and its requests, the input's level metered while the
 * Input tab's meter follows it; none until the session has told it. Without a session, the fixture,
 * changed where it is (its level stays at nothing).
 */
function useSoundModel(): { sound: SoundState | null; actions: SoundActions | undefined } {
  const told: SoundState | null = useSoundState();
  const [fixture, setFixture] = React.useState(SOUND_FIXTURE);
  const session = React.useMemo<SoundActions>(
    () => ({
      select: (direction, id) => {
        systemSound.select(direction, id);
      },
      setVolume: (direction, volume) => {
        systemSound.setVolume(direction, volume);
      },
      setMuted: (direction, muted) => {
        systemSound.setMuted(direction, muted);
      },
      setBalance: (balance) => {
        systemSound.setBalance(balance);
      },
      followLevel: followInputLevel,
    }),
    [],
  );
  const local = React.useMemo<SoundActions>(() => {
    const change = (direction: SoundDirection, side: (side: SoundSide) => SoundSide) =>
      setFixture((state) => ({ ...state, [direction]: side(state[direction]) }));
    return {
      select: (direction, id) =>
        change(direction, (side) => ({
          ...side,
          devices: side.devices.map((device) => ({ ...device, selected: device.id === id })),
        })),
      setVolume: (direction, volume) => change(direction, (side) => ({ ...side, volume })),
      setMuted: (direction, muted) => change(direction, (side) => ({ ...side, muted })),
      setBalance: (balance) => change('output', (side) => ({ ...side, balance })),
      followLevel: () => () => {},
    };
  }, []);
  if (!available) return { sound: fixture, actions: local };
  return { sound: told, actions: told ? session : undefined };
}

/** The reference ThinkPad's battery (docs/adr/0057-battery-and-energy.mdx): 64 %, discharging with
    3 h 20 min left, 87 % of its design after 412 cycles, its charge limit of 75–80 % off. */
const POWER_FIXTURE: PowerState = {
  battery: { percent: 64, state: 'discharging', onAc: false, minutes: 200, low: false },
  health: { maxCapacity: 87, cycles: 412, worn: false },
  chargeLimit: { enabled: false, end: 80, start: 75 },
  lowPower: { mode: 'never', on: false, profiles: true, held: false },
  showPercentage: false,
  criticalAction: 'HybridSleep',
};

/** A day of the fixture's machine: unplugged in the morning, charged at lunch, used all afternoon. */
function fixtureChart(end: number, bin: number, bins: number): HistoryChart {
  const levels: (number | null)[] = [];
  const charging: boolean[] = [];
  const screen: number[] = [];
  for (let index = 0; index < bins; index++) {
    const start = end - (bins - index) * bin;
    const hour = new Date(start * 1000).getHours();
    const awake = hour >= 8 && hour < 23;
    const charges = hour >= 13 && hour < 15;
    const level = !awake ? null : charges ? 40 + (hour - 13) * 30 : hour < 13 ? 100 - (hour - 8) * 12 : 100 - (hour - 15) * 5;
    levels.push(level === null ? null : Math.max(5, Math.min(100, Math.round(level))));
    charging.push(charges);
    screen.push(awake ? Math.round(bin * (hour % 3 === 0 ? 0.5 : 0.85)) : 0);
  }
  return { end, bin, levels, charging, screen };
}

const ENERGY_FIXTURE: AppEnergy[] = [
  { id: 'so.oxy.allo', name: 'Allo', cpu: 1265, gpu: 312, recent: 0.34, significant: true, running: true },
  { id: 'so.oxy.inbox', name: 'Inbox', cpu: 412, gpu: 96, recent: 0.02, significant: false, running: true },
  { id: 'so.oxy.settings', name: 'Settings', cpu: 38, gpu: 11, recent: 0.01, significant: false, running: true },
];

/**
 * Battery and energy (docs/adr/0057-battery-and-energy.mdx): in a session, the state the session
 * tells Settings with its `settings` grant (none until it has: the page waits for it), the history
 * and each app's energy asked as the page shows them, the charge limit asked of the session. Without
 * a session, the reference ThinkPad's.
 */
function usePowerModel(): { power: PowerState | null; actions: PowerActions } {
  const told: PowerState | null = usePowerState();
  const [fixture, setFixture] = React.useState(POWER_FIXTURE);
  const session = React.useMemo<PowerActions>(
    () => ({
      history: async () => historyOf(await systemPower.history()),
      energy: async () => energyOf(await systemPower.energy()),
      setChargeLimit: async (enabled) => chargeLimitFailureOf(await systemPower.setChargeLimit(enabled)),
    }),
    [],
  );
  const local = React.useMemo<PowerActions>(
    () => ({
      history: async (): Promise<PowerHistory> => {
        const now = Math.floor(Date.now() / 1000);
        return {
          day: fixtureChart(Math.ceil(now / 900) * 900, 900, 96),
          days: fixtureChart(Math.ceil(now / 3600) * 3600, 3600, 240),
        };
      },
      energy: async () => ENERGY_FIXTURE,
      setChargeLimit: async (enabled) => {
        setFixture((state) => (state.chargeLimit ? { ...state, chargeLimit: { ...state.chargeLimit, enabled } } : state));
        return null;
      },
    }),
    [],
  );
  if (!available) return { power: fixture, actions: local };
  return { power: told, actions: session };
}

/**
 * Bluetooth (docs/adr/0055-bluetooth.mdx): the state the session tells a client with its
 * `bluetooth` grant, followed, and its requests. The session tells nothing without the grant, so
 * the page is there only once it has told the state. Without a session the fixture plays the
 * machine: discovery finds its nearby devices after a moment, a connection pairs and connects
 * after another (no code asked: that is the shell's sheet), a phone is paired with nothing to
 * connect, and a forget or a rename changes the list.
 */
function useBluetoothModel(): { bluetooth: BluetoothState | undefined; actions: BluetoothActions | undefined } {
  const told: BluetoothState | null = useBluetoothState();
  const [fixture, setFixture] = React.useState(BLUETOOTH_FIXTURE);
  const current = React.useRef(fixture);
  current.current = fixture;
  const session = React.useMemo<BluetoothActions>(
    () => ({
      setPower: (on) => systemBluetooth.setPower(on) as Promise<BluetoothAnswer>,
      discover: (on) => systemBluetooth.discover(on) as Promise<BluetoothAnswer>,
      connect: (id) => systemBluetooth.connect(id) as Promise<BluetoothAnswer>,
      disconnect: (id) => systemBluetooth.disconnect(id) as Promise<BluetoothAnswer>,
      forget: (id) => systemBluetooth.forget(id) as Promise<BluetoothAnswer>,
      rename: (id, name) => systemBluetooth.rename(id, name) as Promise<BluetoothAnswer>,
    }),
    [],
  );
  const local = React.useMemo<BluetoothActions>(() => {
    const later = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const sorted = (devices: BluetoothDevice[]) =>
      [...devices].sort((a, b) => a.name.localeCompare(b.name) || a.address.localeCompare(b.address));
    const change = (id: string, edit: (device: BluetoothDevice) => BluetoothDevice) =>
      setFixture((state) => ({
        ...state,
        devices: sorted(state.devices.map((device) => (device.id === id ? edit(device) : device))),
      }));
    const on = () => !!current.current.adapter?.powered;
    const find = (id: string) => current.current.devices.find((device) => device.id === id);
    return {
      setPower: async (powered) => {
        setFixture((state) => ({
          ...state,
          adapter: state.adapter && {
            ...state.adapter,
            powered,
            discovering: false,
            discoverable: false,
          },
          // Off, every device lets go and nothing nearby is seen; on, the paired ones that were
          // connected as the fixture began come back, as their own devices reconnect them.
          devices: state.devices
            .filter((d) => d.paired)
            .map((d) => ({
              ...d,
              busy: null,
              connected: powered && !!BLUETOOTH_PAIRED.find((p) => p.id === d.id)?.connected,
            })),
        }));
        return { bluetooth: current.current };
      },
      discover: async (wanted) => {
        if (!on()) return { failed: { reason: 'unavailable' } };
        setFixture((state) => ({
          ...state,
          adapter: state.adapter && { ...state.adapter, discovering: wanted, discoverable: wanted },
          devices: wanted ? state.devices : state.devices.filter((d) => d.paired),
        }));
        if (wanted) {
          await later(1200);
          setFixture((state) =>
            state.adapter?.discovering
              ? {
                  ...state,
                  devices: sorted([
                    ...state.devices,
                    ...BLUETOOTH_NEARBY.filter((n) => !state.devices.some((d) => d.id === n.id)),
                  ]),
                }
              : state,
          );
        }
        return { discovering: { on: wanted } };
      },
      connect: async (id) => {
        const device = find(id);
        if (!device) return { failed: { reason: 'notFound' } };
        if (!on()) return { failed: { reason: 'unavailable' } };
        if (device.busy) return { failed: { reason: 'busy' } };
        change(id, (d) => ({ ...d, busy: 'connecting' }));
        await later(1500);
        // A phone pairs with nothing to connect, as a Mac's does.
        const phone = device.kind === 'phone';
        change(id, (d) => ({
          ...d,
          busy: null,
          paired: true,
          connected: !phone,
          battery: d.battery ?? (d.kind === 'speaker' ? 70 : null),
        }));
        return phone ? { paired: { id } } : { connected: { id } };
      },
      disconnect: async (id) => {
        if (!find(id)) return { failed: { reason: 'notFound' } };
        change(id, (d) => ({ ...d, busy: 'disconnecting' }));
        await later(600);
        change(id, (d) => ({ ...d, busy: null, connected: false }));
        return { disconnected: { id } };
      },
      forget: async (id) => {
        if (!find(id)) return { failed: { reason: 'notFound' } };
        setFixture((state) => ({ ...state, devices: state.devices.filter((d) => d.id !== id) }));
        return { forgotten: { id } };
      },
      rename: async (id, typed) => {
        if (!find(id)) return { failed: { reason: 'notFound' } };
        const name = typed.trim() || BLUETOOTH_NAMES.get(id) || id;
        change(id, (d) => ({ ...d, name }));
        return { renamed: { id, name } };
      },
    };
  }, []);
  if (!available) return { bluetooth: fixture, actions: local };
  return told ? { bluetooth: told, actions: session } : { bluetooth: undefined, actions: undefined };
}

/**
 * The input sources (docs/adr/0041-input-methods.mdx, decision 6): in a session, as it tells them
 * (none until it has); without one, the fixture's preferences with every input method installed.
 */
function useInputSources(snapshot: SettingsSnapshot, session: boolean) {
  const told = useInputSourcesState();
  if (!session) {
    const { preferences, system } = snapshot;
    return inputSourcesOf(system.keyboardLayout, preferences.inputSources, preferences.inputSource, INPUT_METHODS);
  }
  return parseInputSources(told);
}

/** The fixture's catalog without a session: the repository's wallpapers would need the host's
    module to be found, so a host outside a session shows the colours alone. */
const FIXTURE_CATALOG: WallpaperCatalog = {
  default: '',
  categories: [],
  colors: [
    { color: '#1f5fd6', name: 'Blue' },
    { color: '#7a3fc2', name: 'Purple' },
    { color: '#2f2f33', name: 'Graphite' },
  ],
};

/** The list of the person's added photos and folders, in Settings' own data (ADR 0028, decision 3):
    none where the application has no folder of its own (a host the session did not start). */
function addedFile(): File | null {
  try {
    return new File(Paths.document, 'wallpapers.json');
  } catch {
    return null;
  }
}

/**
 * Settings › Wallpaper's platform (docs/adr/0048-wallpapers.mdx, decision 7): the system's catalog
 * in the session's language, thumbnails and folders through the host's cache, the system's file
 * chooser through expo-file-system's pickers, and the person's added photos and folders kept in
 * Settings' own data.
 */
function useWallpaperModel(locale: string): {
  catalog: WallpaperCatalog;
  added: AddedWallpaper[];
  actions: WallpaperActions;
} {
  const language = messagesLanguage(locale);
  const catalog = React.useMemo(
    () => (native?.getWallpapers ? (native.getWallpapers(language) ?? EMPTY_CATALOG) : FIXTURE_CATALOG),
    [language],
  );
  const [added, setAddedState] = React.useState<AddedWallpaper[]>(() => {
    const file = addedFile();
    try {
      return file?.exists ? parseAdded(file.textSync()) : [];
    } catch {
      return [];
    }
  });
  const actions = React.useMemo<WallpaperActions>(() => {
    const pickPhoto = async () => {
      try {
        // Expo's answer: `{ canceled }`, or the File chosen as its `result`.
        const picked = await File.pickFileAsync({ mimeTypes: ['image/jpeg', 'image/png', 'image/webp', 'image/svg+xml'] });
        if (picked.canceled) return null;
        const chosen = Array.isArray(picked.result) ? picked.result[0] : picked.result;
        return chosen?.uri ? uriPath(chosen.uri) : null;
      } catch {
        // Cancelled, or refused by the session: nothing chosen.
        return null;
      }
    };
    const pickFolder = async () => {
      try {
        const picked = await Directory.pickDirectoryAsync();
        return picked?.uri ? uriPath(picked.uri) : null;
      } catch {
        return null;
      }
    };
    const setAdded = (list: AddedWallpaper[]) => {
      setAddedState(list);
      const file = addedFile();
      try {
        file?.write(addedText(list));
      } catch (error) {
        console.warn('[settings] the added wallpapers were not kept:', error);
      }
    };
    return {
      thumbnail: native?.wallpaperThumbnail ? (uri, w, h) => native.wallpaperThumbnail!(uri, w, h) : undefined,
      folder: native?.wallpaperFolder ? (path) => native.wallpaperFolder!(path) : undefined,
      pickPhoto,
      pickFolder,
      setAdded,
    };
  }, []);
  return { catalog, added, actions };
}

/** The system's folder chooser (docs/adr/0028-file-access.mdx), through expo-file-system's picker:
    the folder's `file://` URI, or null when cancelled or refused. Settings › Spotlight's privacy. */
async function chooseFolder(): Promise<string | null> {
  try {
    const picked = await Directory.pickDirectoryAsync();
    return picked?.uri ?? null;
  } catch {
    return null;
  }
}

type SessionHost = { snapshot(): unknown; subscribe(listener: () => void): () => void };
const sessionHost = (globalThis as Record<symbol, unknown>)[Symbol.for('oxy.so/session-host')] as
  | SessionHost
  | undefined;
const accountSession = TurboModuleRegistry.get<{
  localSignIn(): Promise<unknown>;
  changeLocal(change: string, secrets: { pin?: string; password?: string; currentPin?: string }): Promise<unknown>;
  remove(): Promise<unknown>;
  erasePreview(): Promise<unknown>;
  eraseAll(password: string): Promise<unknown>;
}>('OxyOSAccountSession');
const outcome = (answer: unknown, done: (answer: unknown) => boolean): AccountOutcome => {
  if (done(answer)) return { done: true };
  const reason = (answer as { refused?: { reason?: string }; failed?: { reason?: string } } | null);
  return { failed: reason?.refused?.reason ?? reason?.failed?.reason ?? 'failed' };
};
const accountActions: AccountActions = {
  signOut: async () => outcome(await systemSession.signOut(), (answer) => !!(answer as { signingOut?: unknown })?.signingOut),
  remove: async () => outcome(accountSession ? await accountSession.remove() : null, (answer) => answer === true),
  // The account's sign-in on this computer (docs/adr/0020-oxy-account-login.mdx decision 2): its
  // PIN and password, read and changed by the accounts' service; the secrets go to the host, which
  // sends them on and forgets them.
  local: accountSession
    ? {
        read: async () => localSignInFrom(await accountSession.localSignIn()),
        change: async (change, secrets) => localOutcomeFrom(await accountSession.changeLocal(change, secrets)),
      }
    : undefined,
};

/** Erasing the computer, the accounts' service's (Settings alone may ask): what it would do, read
    strictly, and the erase with the administrator's password, handed to the host, which sends it
    to the service and forgets it. */
const eraseActions: EraseActions | undefined = accountSession
  ? {
      preview: async () => erasePreviewFrom(await accountSession.erasePreview()),
      eraseAll: async (password) => eraseOutcomeFrom(await accountSession.eraseAll(password)),
    }
  : undefined;

/** The machine's Oxy session as the platform's session host tells it (none outside a session). */
function useAccount(): AccountSnapshot | null {
  const told = React.useSyncExternalStore(
    (listener) => sessionHost?.subscribe(listener) ?? (() => {}),
    () => sessionHost?.snapshot() ?? null,
  );
  return React.useMemo(() => accountSnapshotFrom(told), [told]);
}

/**
 * The page the session started Settings to show (docs/adr/0019-settings.mdx): its launch URL's,
 * `oxyos-settings://PAGE`, which the shell or an application opened (an application's request to
 * manage the Oxy account, `oxyos-settings://oxy-account`); none for a plain launch.
 */
function launchPage(): string | undefined {
  const url: unknown = TurboModuleRegistry.get<any>('OxyOSApplication')?.getConstants().launchURL;
  const match = typeof url === 'string' ? /^oxyos-settings:\/\/([a-z0-9-]+)\/?$/i.exec(url) : null;
  return match ? match[1]!.toLowerCase() : undefined;
}

export default function SettingsApp() {
  const model = useSettingsModel();
  const install = useInstallModel();
  const networks = useNetworkModel();
  const bluetooth = useBluetoothModel();
  const sound = useSoundModel();
  const power = usePowerModel();
  const inputSources = useInputSources(model.snapshot, model.session);
  const snapshot: SettingsSnapshot = {
    ...model.snapshot,
    updates: install.updates,
    privacy: install.privacy,
    network: networks.network,
    bluetooth: bluetooth.bluetooth,
    sound: sound.sound,
    power: power.power,
    inputSources,
    account: useAccount(),
  };
  const actions: SettingsActions = {
    ...model.actions,
    updates: install.updatesActions,
    privacy: install.privacyActions,
    network: networks.actions,
    bluetooth: bluetooth.actions,
    sound: sound.actions,
    power: power.actions,
    account: sessionHost ? accountActions : undefined,
    chooseFolder,
    // Settings › Spotlight (docs/adr/0051-search.mdx): the session's search index, which takes its
    // preferences from Settings alone.
    search: oxyosSurfaces.search,
    erase: sessionHost ? eraseActions : undefined,
  };
  // The session's locale, as it tells every client; the fixture's from its own preferences.
  const told = useSessionLocale();
  const locale =
    (model.session && told) || sessionLocale(model.snapshot.preferences, model.snapshot.system.locale);
  const wallpapers = useWallpaperModel(locale);
  snapshot.wallpapers = wallpapers.catalog;
  snapshot.addedWallpapers = wallpapers.added;
  actions.wallpaper = wallpapers.actions;
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale={locale}>
        <Settings snapshot={snapshot} actions={actions} initialPage={launchPage()} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
