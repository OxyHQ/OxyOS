/**
 * An application that quits unexpectedly while its own data is corrupt
 * (docs/adr/0061-crashes-and-resetting-an-application.mdx), installed as an application of its own
 * for the crash scenarios (scenarios/app-crash*.json, app-reset*.json). It shows its window, then
 * reads its storage (AsyncStorage, its data directory's `async-storage.json`): when that cannot be
 * read, as a corrupted preferences file cannot, once its window is the active one the error is
 * thrown where nothing catches it, and React Native's fatal error ends the application, as it ends
 * any. Reset, its data is gone: it reads nothing, counts its first launch and says it is ready.
 */
import * as React from 'react';
import { DeviceEventEmitter, Text, TurboModuleRegistry, View } from 'react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';

/** Its window's state, as the Bloom host tells it (`OxyOSWindow`, docs/adr/0016-windows.mdx). */
const windowModule = TurboModuleRegistry.get('OxyOSWindow') as { getState(): { active?: boolean } } | null;

/** `then` once its window is the active one: it quits from a window the person saw, every time the
    same way, never before its window could be shown. */
function whenActive(then: () => void) {
  if (windowModule?.getState?.()?.active) {
    then();
    return;
  }
  const told = DeviceEventEmitter.addListener('OxyOSWindowChanged', (state: { active?: boolean }) => {
    if (!state?.active) return;
    told.remove();
    then();
  });
}

export default function CrashProbe() {
  const [said, setSaid] = React.useState('Reading…');
  React.useEffect(() => {
    AsyncStorage.getItem('launches').then(
      async (launches) => {
        const now = Number(launches ?? 0) + 1;
        await AsyncStorage.setItem('launches', String(now));
        setSaid(`Ready, launch ${now}`);
      },
      (error: Error) =>
        whenActive(() => {
          // Nothing catches it: a timer's error is React Native's fatal one.
          setTimeout(() => {
            throw new Error(`Its storage cannot be read: ${error.message}`);
          }, 0);
        }),
    );
  }, []);
  return (
    <View testID="crash-probe" style={{ flex: 1, backgroundColor: 'white', padding: 20 }}>
      <Text>{said}</Text>
    </View>
  );
}
