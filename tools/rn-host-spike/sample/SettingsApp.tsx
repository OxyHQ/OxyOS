/**
 * OxyOS's Settings (apps/settings) in the Bloom host (docs/adr/0019-settings.mdx). Its model is the
 * session's: every preference as the session keeps it (`OxyOSSettings`, which only the client the
 * session granted `settings` has), a change asked of the session, which validates, keeps and
 * applies it and tells every application; the displays as the session shows them and their changes
 * (docs/adr/0018-several-displays.mdx); and the machine's facts. Without the grant (a host run
 * outside a session, the browser preview) the model is a fixture kept in memory.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Settings } from '../../../apps/settings/src/Settings';
import { stringsFor } from '../../../apps/settings/src/strings';
import { available, privacy, updates as systemUpdates, useUpdateState } from './oxyos/system';
import type { PrivacyApplication, UpdatesActions } from '../../../apps/settings/src/updates';
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

export default function SettingsApp() {
  const model = useSettingsModel();
  const install = useInstallModel();
  const snapshot = { ...model.snapshot, updates: install.updates, privacy: install.privacy };
  const actions = { ...model.actions, updates: install.updatesActions, privacy: install.privacyActions };
  const strings = React.useMemo(() => stringsFor(Intl.DateTimeFormat().resolvedOptions().locale), []);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces}>
        <Settings snapshot={snapshot} actions={actions} strings={strings} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
