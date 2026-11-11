/**
 * About This Computer (apps/settings/src/AboutThisComputer.tsx) in the Bloom host: the Oxy menu's
 * first row, as a Mac's About This Mac. Its facts are the ones Settings › About shows, read where
 * Settings reads them (`OxyOSSettings.getSystem`: /etc/hostname, os-release and /proc, read only,
 * needing no grant); More Info… opens Settings there (`oxyos-settings://about`), as any application
 * opens Settings at a page. Its language is the session's locale (docs/adr/0040-language-and-region.mdx,
 * decision 5).
 */
import * as React from 'react';
import { Linking, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { useSessionLocale } from './oxyos/locale';
import { AboutThisComputer } from '../../../apps/settings/src/AboutThisComputer';
import { messagesFor } from '../../../apps/settings/src/messages';
import { MORE_INFO_URL, type SystemFacts } from '../../../apps/settings/src/model';

const native = TurboModuleRegistry.get<{ getSystem(): SystemFacts }>('OxyOSSettings');

/** Outside a host (the browser preview), nothing is known of the machine: no fact is shown. */
const UNKNOWN: SystemFacts = {
  computerName: '',
  version: '',
  processor: '',
  memoryBytes: 0,
  display: '',
  timeZone: '',
  automaticTime: false,
  keyboardLayout: '',
  locale: '',
};

export default function AboutThisComputerApp() {
  const locale = useSessionLocale();
  const system = React.useMemo(() => native?.getSystem() ?? UNKNOWN, []);
  const moreInfo = React.useCallback(() => {
    Linking.openURL(MORE_INFO_URL).catch(() => {});
  }, []);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale={locale}>
        <AboutThisComputer system={system} messages={messagesFor(locale)} moreInfo={moreInfo} />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
