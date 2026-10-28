/**
 * OxyOS side of the Expo modules Bloom reaches (expo-font, and expo-asset beneath it).
 * expo-modules-core's own global polyfill provides the module classes; the native work is done
 * by the host's OxyFontRegistry TurboModule. Anything OxyOS cannot do fails loudly.
 */
import { installExpoGlobalPolyfill } from 'expo-modules-core/src/polyfill/dangerous-internal';
import { TurboModuleRegistry } from 'react-native';

installExpoGlobalPolyfill();

const fonts = TurboModuleRegistry.getEnforcing('OxyFontRegistry');
const loaded = new Set();

globalThis.expo.modules.ExpoFontLoader = {
  getLoadedFonts: () => [...loaded],
  isLoaded: (family) => loaded.has(family),
  loadAsync: async (family, uri) => {
    if (!fonts.registerFont(family, String(uri))) {
      throw new Error(`OxyOS could not load font "${family}" from ${uri}`);
    }
    loaded.add(family);
  },
};

globalThis.expo.modules.ExpoAsset = {
  // Bundled assets are already files next to the bundle; OxyOS has no asset download service.
  downloadAsync: async (url) => {
    if (String(url).startsWith('file://')) return url;
    throw new Error(`OxyOS cannot download asset ${url}`);
  },
};
