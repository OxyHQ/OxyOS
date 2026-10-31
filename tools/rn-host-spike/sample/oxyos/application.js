/**
 * An Oxy application from its own checkout (`OXYOS_APPLICATION_DIR`, oxyos/application-build.js),
 * run as `expo-router/entry` runs it: Stash, the file manager, Atlas, the App Store (ADR 0030,
 * decision 13). metro.config.js resolves this module's requests as the application's root would
 * make them, and makes it an empty module when the bundle has no application.
 *
 * `expo-router/entry` loads `@expo/metro-runtime`, then registers the App of expo-router's
 * qualified entry (`ExpoRoot` over `expo-router/_ctx`, the require.context of the application's
 * `app/` directory) with `expo`'s `registerRootComponent`, as AppRegistry's `main`. Here it is the
 * module the host was started to run (`OxyOSApplication`'s `module`: the registry entry's
 * `X-OxyOS-Bloom-Module`, its package's `entry.module`), and what the runtime owns is
 * around it: Bloom's platform adapter (ADR 0015), which the application's own BloomProvider keeps,
 * and, for an application the image's policy gives `store`, the machine's installer in its Oxy
 * SDK (oxyos/store-registration.js). Everything else (gesture handler root, safe area, Bloom, the
 * Oxy SDK) is the application's `app/_layout`.
 *
 * Its splash screen: OxyOS has none (no ExpoSplashScreen module), so expo-router's calls to it do
 * nothing, as on the web.
 */
import '@expo/metro-runtime';
// registerRootComponent's own first import: Expo's runtime set-up (`expo/src/Expo.fx`).
import 'expo';
// Before the application's code runs, as @oxy.so/core asks of a runtime that installs.
import './store-registration';
import { App } from 'expo-router/build/qualified-entry';
import { PlatformSurfaceProvider } from '@oxy.so/bloom/platform-surface';
import { AppRegistry, TurboModuleRegistry } from 'react-native';
import { oxyosSurfaces } from './platform-surface';

function Application() {
  return (
    <PlatformSurfaceProvider value={oxyosSurfaces}>
      <App />
    </PlatformSurfaceProvider>
  );
}

AppRegistry.registerComponent(
  TurboModuleRegistry.getEnforcing('OxyOSApplication').getConstants().module,
  () => Application,
);
