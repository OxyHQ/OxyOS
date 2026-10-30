/**
 * OxyOS's Settings (apps/settings) in the Bloom host (docs/adr/0019-settings.mdx). Its model is the
 * session's: every preference as the session keeps it (`OxyOSSettings`, which only the client the
 * session granted `settings` has), a change asked of the session, which validates, keeps and
 * applies it and tells every application; and the machine's facts. Without the grant (a host run
 * outside a session, the browser preview) the model is a fixture kept in memory.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Settings } from '../../../apps/settings/src/Settings';
import { stringsFor } from '../../../apps/settings/src/strings';
import {
  PREFERENCE_KEYS,
  preferenceText,
  type Appearance,
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
  const [fixture, setFixture] = React.useState(FIXTURE);
  React.useEffect(() => {
    const subscription = DeviceEventEmitter.addListener('OxyOSSettingsChanged', setValues);
    // A change between the first read and the subscription is not lost.
    setValues(native?.getValues() ?? null);
    return () => subscription.remove();
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
    },
    actions,
  };
}

export default function SettingsApp() {
  const { snapshot, actions } = useSettingsModel();
  const strings = React.useMemo(() => stringsFor(Intl.DateTimeFormat().resolvedOptions().locale), []);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces}>
        <Settings snapshot={snapshot} actions={actions} strings={strings} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
