/**
 * Stash (OxyHQ/Stash), OxyOS's file manager: its own Expo Router root, from its checkout
 * (`STASH_DIR`, oxyos/application-build.js), run as `expo-router/entry` runs it. metro.config.js
 * resolves this module's requests as the application's root would make them, and makes it an
 * empty module when the bundle has no application.
 *
 * `expo-router/entry` loads `@expo/metro-runtime`, then registers the App of expo-router's
 * qualified entry (`ExpoRoot` over `expo-router/_ctx`, the require.context of the application's
 * `app/` directory) with `expo`'s `registerRootComponent`, as AppRegistry's `main`. Here it is the
 * session's module `Stash` (the registry entry's `X-OxyOS-Bloom-Module`), and what the runtime
 * owns is around it: Bloom's platform adapter (ADR 0015), which the application's own
 * BloomProvider keeps. Everything else (gesture handler root, safe area, Bloom, the Oxy SDK) is
 * the application's `app/_layout`.
 *
 * Its splash screen: OxyOS has none (no ExpoSplashScreen module), so expo-router's calls to it do
 * nothing, as on the web.
 */
import '@expo/metro-runtime';
// registerRootComponent's own first import: Expo's runtime set-up (`expo/src/Expo.fx`).
import 'expo';
import { App } from 'expo-router/build/qualified-entry';
import { PlatformSurfaceProvider } from '@oxy.so/bloom/platform-surface';
import { AppRegistry } from 'react-native';
import { oxyosSurfaces } from './platform-surface';

function Stash() {
  return (
    <PlatformSurfaceProvider value={oxyosSurfaces}>
      <App />
    </PlatformSurfaceProvider>
  );
}

AppRegistry.registerComponent('Stash', () => Stash);
