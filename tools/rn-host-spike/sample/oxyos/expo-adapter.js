/**
 * OxyOS side of the Expo modules Bloom and Oxy apps reach (expo-font, expo-asset beneath it,
 * expo-clipboard, and expo-blur's views). expo-modules-core's own global polyfill provides the
 * module classes; the native work is done by the host's TurboModules (OxyOSFontRegistry,
 * OxyOSClipboard) and host components (expo-blur.h). Anything OxyOS cannot do fails loudly.
 */
import { installExpoGlobalPolyfill } from 'expo-modules-core/src/polyfill/dangerous-internal';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

installExpoGlobalPolyfill();

const fonts = TurboModuleRegistry.getEnforcing('OxyOSFontRegistry');
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

// expo-clipboard, on the OS clipboard (docs/adr/0007-clipboard.mdx). Reading and writing it use
// the serial of the latest key or press the app received, so the compositor refuses both unless
// the app has keyboard focus and acted just before: a read refused that way gives "".
const clipboard = TurboModuleRegistry.getEnforcing('OxyOSClipboard');
const isUrl = (text) => /^[a-z][a-z0-9+.-]*:\/\/\S+$/i.test(text.trim());
const noImages = async () => {
  throw new Error('Images on the clipboard are not supported on OxyOS yet');
};
const ExpoClipboard = new globalThis.expo.EventEmitter();
Object.assign(ExpoClipboard, {
  // Plain text only: HTML is offered as no other type on OxyOS yet.
  getStringAsync: () => clipboard.getString(),
  setStringAsync: (text) => clipboard.setString(String(text)),
  hasStringAsync: () => clipboard.hasString(),
  getUrlAsync: async () => {
    const text = await clipboard.getString();
    return isUrl(text) ? text.trim() : null;
  },
  setUrlAsync: (url) => clipboard.setString(String(url)),
  hasUrlAsync: async () => isUrl(await clipboard.getString()),
  getImageAsync: noImages,
  setImageAsync: noImages,
  hasImageAsync: noImages,
});
DeviceEventEmitter.addListener('OxyOSClipboardChanged', ({ text }) =>
  ExpoClipboard.emit('onClipboardChanged', { contentTypes: text ? ['plain-text'] : [] }),
);
globalThis.expo.modules.ExpoClipboard = ExpoClipboard;

// Expo's native views the host draws (expo-blur.h), by module and view, with the props each takes
// as its Android view definition names them; React Native's view props come with every view.
const expoViews = {
  'ExpoBlur.ExpoBlurView': ['blurTargetId', 'intensity', 'tint', 'blurReductionFactor', 'borderRadii', 'blurMethod'],
  'ExpoBlur.ExpoBlurTargetView': [],
};
globalThis.expo.getViewConfig = (moduleName, viewName) => {
  const props = expoViews[`${moduleName}.${viewName}`];
  if (!props) throw new Error(`Expo's ${moduleName} ${viewName ?? 'view'} is not available on OxyOS`);
  return { validAttributes: Object.fromEntries(props.map((prop) => [prop, true])), directEventTypes: {} };
};
