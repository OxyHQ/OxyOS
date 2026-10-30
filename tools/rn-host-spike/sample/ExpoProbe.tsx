/**
 * What the platform modules an Expo application reaches do on OxyOS (docs/react-native-android-adaptation.mdx,
 * application.h, oxyos/expo-adapter.js), through each package's own API, as an application calls
 * them: AsyncStorage kept in the application's data directory (a value kept by an earlier run read
 * back, values written, merged, listed and removed), NetInfo's connectivity, expo-crypto's
 * digests and random values, expo-secure-store keeping nothing, expo-haptics without hardware,
 * Linking and expo-linking with no link and none to open, and expo-constants with no app config
 * of its own. Each row either works, or rejects or throws with its reason; none is left pending.
 * scenarios/expo-modules.json checks every row and the storage file.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import AsyncStorage from '@react-native-async-storage/async-storage';
import NetInfo from '@react-native-community/netinfo';
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import * as Haptics from 'expo-haptics';
import * as ExpoLinking from 'expo-linking';
import Constants from 'expo-constants';
import { Linking } from 'react-native';

type Outcome = string;

const TIMEOUT_MS = 2000;

function settle(run: () => unknown): Promise<Outcome> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve('pending'), TIMEOUT_MS);
    const done = (outcome: Outcome) => {
      clearTimeout(timer);
      resolve(outcome);
    };
    try {
      Promise.resolve(run()).then(
        (value) => done(`ok ${JSON.stringify(value) ?? ''}`.trim()),
        (error: Error) => done(`rejected: ${String(error?.message ?? error).split('\n')[0]}`),
      );
    } catch (error) {
      done(`threw: ${String((error as Error)?.message ?? error).split('\n')[0]}`);
    }
  });
}

// In order: the storage rows build on each other.
const PROBES: [string, () => unknown][] = [
  ['AsyncStorage.getItem(kept)', () => AsyncStorage.getItem('kept')],
  [
    'AsyncStorage.setItem',
    async () => {
      await AsyncStorage.multiSet([
        ['written', 'now'],
        ['user', JSON.stringify({ name: 'Ada', prefs: { dark: true } })],
      ]);
      return AsyncStorage.getItem('written');
    },
  ],
  [
    'AsyncStorage.mergeItem',
    async () => {
      await AsyncStorage.mergeItem('user', JSON.stringify({ prefs: { size: 2 } }));
      return JSON.parse((await AsyncStorage.getItem('user')) ?? 'null');
    },
  ],
  ['AsyncStorage.mergeItem(text)', () => AsyncStorage.mergeItem('written', JSON.stringify({ a: 1 }))],
  [
    'AsyncStorage.removeItem',
    async () => {
      await AsyncStorage.removeItem('kept');
      return [...(await AsyncStorage.getAllKeys())].sort();
    },
  ],
  [
    'NetInfo.fetch',
    async () => {
      const state = await NetInfo.fetch();
      return `${typeof state.isConnected} ${['none', 'wifi', 'ethernet', 'cellular', 'vpn', 'other'].includes(state.type)}`;
    },
  ],
  ['Crypto.digestStringAsync(SHA-256)', () => Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA256, 'abc')],
  [
    'Crypto.digestStringAsync(SHA-1, base64)',
    () =>
      Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.SHA1, 'abc', {
        encoding: Crypto.CryptoEncoding.BASE64,
      }),
  ],
  ['Crypto.digestStringAsync(MD5)', () => Crypto.digestStringAsync(Crypto.CryptoDigestAlgorithm.MD5, 'abc')],
  [
    'Crypto.digest(SHA-512)',
    async () => new Uint8Array(await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA512, new Uint8Array([97, 98, 99]))).length,
  ],
  [
    'Crypto.getRandomBytes',
    () => {
      const a = Crypto.getRandomBytes(32);
      const b = Crypto.getRandomBytes(32);
      return `${a.length} ${a.some((byte, i) => byte !== b[i])}`;
    },
  ],
  ['Crypto.randomUUID', () => /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(Crypto.randomUUID())],
  ['SecureStore.getItemAsync', () => SecureStore.getItemAsync('credential')],
  ['SecureStore.setItemAsync', () => SecureStore.setItemAsync('credential', 'secret')],
  ['Haptics.impactAsync', () => Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)],
  ['Linking.getInitialURL', () => Linking.getInitialURL()],
  ['Linking.canOpenURL', () => Linking.canOpenURL('https://oxy.so')],
  ['ExpoLinking.getLinkingURL', () => ExpoLinking.getLinkingURL()],
  ['Constants.expoConfig', () => Constants.expoConfig],
  ['Constants.executionEnvironment', () => Constants.executionEnvironment],
];

export default function ExpoProbe() {
  const [outcomes, setOutcomes] = React.useState<Record<string, Outcome>>({});
  React.useEffect(() => {
    (async () => {
      for (const [name, run] of PROBES) {
        const outcome = await settle(run);
        setOutcomes((all) => ({ ...all, [name]: outcome }));
      }
    })();
  }, []);
  return (
    <View style={{ flex: 1, padding: 12, gap: 2, backgroundColor: '#fff' }}>
      {PROBES.map(([name]) => (
        <Text key={name} testID={`probe-${name}`} style={{ fontSize: 12 }}>
          {`${name}: ${outcomes[name] ?? 'running'}`}
        </Text>
      ))}
    </View>
  );
}
