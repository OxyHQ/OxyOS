/**
 * What React Native's Android APIs do on OxyOS. React Native's own modules run their Android
 * implementation here (metro.config.js), but OxyOS is not Android: each API either works, is
 * unavailable (its native module does not exist), or rejects. None may pretend to succeed, and
 * none may leave its caller waiting: an answer still pending after 2 s is shown as `pending`.
 * scenarios/android-adaptation.json checks every row. Its ActivityIndicator is React Native's
 * Android `AndroidProgressBar`, a native component the host does not implement: drawn as a plain
 * view and said (unimplemented-components.h), which the scenario declares.
 */
import * as React from 'react';
import { ActivityIndicator, Text, View } from 'react-native';

type Outcome = string;

const TIMEOUT_MS = 2000;

type Handler = (error: unknown, isFatal?: boolean) => void;
const errorUtils = (globalThis as { ErrorUtils?: { getGlobalHandler(): Handler; setGlobalHandler(h: Handler): void } })
  .ErrorUtils;

/**
 * `run()`'s result, with what React Native reported meanwhile. A module whose own code throws as it
 * loads (a native module OxyOS does not have, read with `getEnforcing`) is reported by React
 * Native's require as a fatal error, which ends an application
 * (docs/adr/0061-crashes-and-resetting-an-application.mdx), and returns nothing: the probe takes
 * those reports itself while each probe runs, as an application with its own error handler does,
 * so that it can show every module's answer in one run.
 */
function running(run: () => unknown): { result: unknown; reported: unknown } {
  let reported: unknown = undefined;
  const handler = errorUtils?.getGlobalHandler();
  errorUtils?.setGlobalHandler((error) => {
    reported = error;
  });
  try {
    return { result: run(), reported };
  } catch (error) {
    // What failed after the report (the module read as nothing) is said by the report itself.
    throw reported ?? error;
  } finally {
    if (handler) errorUtils?.setGlobalHandler(handler);
  }
}

function settle(run: () => unknown): Promise<Outcome> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve('pending'), TIMEOUT_MS);
    const done = (outcome: Outcome) => {
      clearTimeout(timer);
      resolve(outcome);
    };
    try {
      const { result, reported } = running(run);
      if (reported !== undefined) throw reported;
      if (result && typeof (result as Promise<unknown>).then === 'function') {
        (result as Promise<unknown>).then(
          (value) => done(`ok ${JSON.stringify(value) ?? ''}`.trim()),
          (error: Error) => done(`rejected: ${String(error?.message ?? error).split('\n')[0]}`),
        );
      } else {
        done(`ok ${JSON.stringify(result) ?? ''}`.trim());
      }
    } catch (error) {
      // A native module that does not exist on OxyOS fails on access, whatever its message.
      done(`unavailable (${String((error as Error)?.message ?? error).split('\n')[0]})`);
    }
  });
}

// Each reads its module from `react-native` lazily: a missing native module throws on access.
const PROBES: Record<string, () => unknown> = {
  ToastAndroid: () => require('react-native').ToastAndroid.show('probe', 0),
  Vibration: () => require('react-native').Vibration.vibrate(10),
  PermissionsAndroid: () => require('react-native').PermissionsAndroid.check('android.permission.CAMERA'),
  'Linking.openURL': () => require('react-native').Linking.openURL('https://oxy.so'),
  Share: () => require('react-native').Share.share({ message: 'probe' }),
  'AccessibilityInfo.getRecommendedTimeoutMillis': () =>
    require('react-native').AccessibilityInfo.getRecommendedTimeoutMillis(1000),
  'AccessibilityInfo.isScreenReaderEnabled': () =>
    require('react-native').AccessibilityInfo.isScreenReaderEnabled(),
  'AccessibilityInfo.isAccessibilityServiceEnabled': () =>
    require('react-native').AccessibilityInfo.isAccessibilityServiceEnabled(),
  'AccessibilityInfo.isReduceTransparencyEnabled': () =>
    require('react-native').AccessibilityInfo.isReduceTransparencyEnabled(),
  'Appearance.getColorScheme': () => require('react-native').Appearance.getColorScheme(),
  'Dimensions.window': () => require('react-native').Dimensions.get('window').width,
  'I18nManager.isRTL': () => require('react-native').I18nManager.isRTL,
  'PlatformConstants.getAndroidID': () =>
    require('react-native/Libraries/Utilities/NativePlatformConstantsAndroid').default.getAndroidID(),
  'Alert.alert': () => require('react-native').Alert.alert('probe', 'probe', [{ text: 'OK' }]),
  'Alert.prompt': () => require('react-native').Alert.prompt('probe'),
  'Image.getSize': () => require('react-native').Image.getSize('data:image/png;base64,iVBORw0KGgo='),
  'AppState.currentState': () => require('react-native').AppState.currentState,
  // Accepted with no effect: nothing to apply on OxyOS, or a capability it does not have yet.
  'StatusBar.setHidden': () => require('react-native').StatusBar.setHidden(true),
  'I18nManager.forceRTL': () => require('react-native').I18nManager.forceRTL(true),
  'Appearance.setColorScheme': () => require('react-native').Appearance.setColorScheme('dark'),
  'AccessibilityInfo.announceForAccessibility': () =>
    require('react-native').AccessibilityInfo.announceForAccessibility('probe'),
};

export default function AndroidProbe() {
  const [outcomes, setOutcomes] = React.useState<Record<string, Outcome>>({});
  React.useEffect(() => {
    for (const [name, run] of Object.entries(PROBES)) {
      settle(run).then((outcome) => setOutcomes((all) => ({ ...all, [name]: outcome })));
    }
  }, []);
  return (
    <View style={{ flex: 1, padding: 16, gap: 4, backgroundColor: '#fff' }}>
      {Object.keys(PROBES).map((name) => (
        <Text key={name} testID={`probe-${name}`}>
          {`${name}: ${outcomes[name] ?? 'running'}`}
        </Text>
      ))}
      <ActivityIndicator testID="progress" />
    </View>
  );
}
