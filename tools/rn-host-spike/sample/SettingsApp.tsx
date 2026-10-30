/**
 * OxyOS's Settings (apps/settings) in the Bloom host. Until the session serves its preferences
 * (`OxyOSSettings`, docs/adr/0019-settings.mdx step 2), its model is a fixture kept in memory:
 * changes show at once and go nowhere.
 */
import * as React from 'react';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Settings } from '../../../apps/settings/src/Settings';
import { stringsFor } from '../../../apps/settings/src/strings';
import type { Preferences, SettingsSnapshot } from '../../../apps/settings/src/model';

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
    keyboardLayout: 'English (US)',
  },
};

export default function SettingsApp() {
  const [snapshot, setSnapshot] = React.useState(FIXTURE);
  const strings = React.useMemo(() => stringsFor(Intl.DateTimeFormat().resolvedOptions().locale), []);
  const actions = React.useMemo(
    () => ({
      setPreference: <K extends keyof Preferences>(key: K, value: Preferences[K]) =>
        setSnapshot((current) => ({ ...current, preferences: { ...current.preferences, [key]: value } })),
    }),
    [],
  );
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Settings snapshot={snapshot} actions={actions} strings={strings} />
    </BloomProvider>
  );
}
