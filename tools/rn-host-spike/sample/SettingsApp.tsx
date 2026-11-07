/**
 * OxyOS's Settings (apps/settings) in the Bloom host (docs/adr/0019-settings.mdx). Its model is the
 * session's: every preference as the session keeps it (`OxyOSSettings`, which only the client the
 * session granted `settings` has), a change asked of the session, which validates, keeps and
 * applies it and tells every application; the displays as the session shows them and their changes
 * (docs/adr/0018-several-displays.mdx); the machine's facts; and the networks as the session
 * tells them (docs/adr/0038-networks.mdx). Without the grant (a host run outside a session, the
 * browser preview) the model is a fixture kept in memory. Its language is the session's locale
 * (docs/adr/0040-language-and-region.mdx, decision 5), given to `BloomProvider` and followed live;
 * the fixture's is the one its own preferences give. Its Oxy account is the machine's Oxy session
 * as the platform gives every application (oxyos/session-host.js, docs/adr/0020-oxy-account-login.mdx):
 * signing out is the session's (`signOut`, with the `settings` grant), removing the account the
 * accounts' service's, which only Settings may ask.
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
  network as systemNetwork,
  privacy,
  sound as systemSound,
  updates as systemUpdates,
  useInputLevel,
  useInputSourcesState,
  useNetworkState,
  useSoundState,
  useUpdateState,
} from './oxyos/system';
import type { PrivacyApplication, UpdateOutcome, UpdatesActions } from '../../../apps/settings/src/updates';
import {
  accountSnapshotFrom,
  type AccountActions,
  type AccountOutcome,
  type AccountSnapshot,
} from '../../../apps/settings/src/account';
import type { NetworkActions, NetworkAnswer, NetworkState } from '../../../apps/settings/src/network';
import type { SoundActions, SoundDirection, SoundSide, SoundState } from '../../../apps/settings/src/sound';
import {
  INPUT_METHODS,
  inputSourcesOf,
  parseInputSources,
  parseInputSourcesPreference,
} from '../../../apps/settings/src/input-sources';
import {
  PREFERENCE_KEYS,
  preferenceText,
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
    inputSources: [],
    inputSource: 'layout',
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
    // None added, the layout current, unless the session keeps others (docs/adr/0041-input-methods.mdx).
    inputSources: parseInputSourcesPreference(values[PREFERENCE_KEYS.inputSources]),
    inputSource: values[PREFERENCE_KEYS.inputSource] || 'layout',
  };
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
  const applications = React.useMemo(() => native?.getApplications?.() ?? [], []);
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
 * Input tab asks for it; none until the session has told it. Without a session, the fixture,
 * changed where it is (its level stays at nothing).
 */
function useSoundModel(): { sound: SoundState | null; level: number; actions: SoundActions | undefined } {
  const told: SoundState | null = useSoundState();
  const [metering, setMetering] = React.useState(false);
  const level: number = useInputLevel(metering);
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
      meter: setMetering,
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
      meter: () => {},
    };
  }, []);
  if (!available) return { sound: fixture, level: 0, actions: local };
  return { sound: told, level, actions: told ? session : undefined };
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

type SessionHost = { snapshot(): unknown; subscribe(listener: () => void): () => void };
const sessionHost = (globalThis as Record<symbol, unknown>)[Symbol.for('oxy.so/session-host')] as
  | SessionHost
  | undefined;
const accountSession = TurboModuleRegistry.get<{ remove(): Promise<unknown> }>('OxyOSAccountSession');
const outcome = (answer: unknown, done: (answer: unknown) => boolean): AccountOutcome => {
  if (done(answer)) return { done: true };
  const reason = (answer as { refused?: { reason?: string }; failed?: { reason?: string } } | null);
  return { failed: reason?.refused?.reason ?? reason?.failed?.reason ?? 'failed' };
};
const accountActions: AccountActions = {
  signOut: async () => outcome(await systemSession.signOut(), (answer) => !!(answer as { signingOut?: unknown })?.signingOut),
  remove: async () => outcome(accountSession ? await accountSession.remove() : null, (answer) => answer === true),
};

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
  const sound = useSoundModel();
  const inputSources = useInputSources(model.snapshot, model.session);
  const snapshot: SettingsSnapshot = {
    ...model.snapshot,
    updates: install.updates,
    privacy: install.privacy,
    network: networks.network,
    sound: sound.sound,
    soundInputLevel: sound.level,
    inputSources,
    account: useAccount(),
  };
  const actions: SettingsActions = {
    ...model.actions,
    updates: install.updatesActions,
    privacy: install.privacyActions,
    network: networks.actions,
    sound: sound.actions,
    account: sessionHost ? accountActions : undefined,
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
