// The runtime's framework, started (docs/adr/0030-installing-applications.mdx, decision 17): run
// once in every application's JavaScript runtime, after React Native's InitializeCore and before the
// application's own bundle.
//
// - Expo's native side, as the host provides it (expo-adapter.js, expo-file-system.js), for every
//   application.
// - The framework's assets (Bloom's fonts, React Native's images) are the runtime's, beside its
//   framework, not the application's: an asset the framework's build marked (`oxyosFramework`,
//   framework-asset-plugin.js) is resolved against the framework's URL, which the host gives
//   (`OxyOSRuntime.frameworkURL`); every other asset against the application's bundle, as React
//   Native resolves it.
import './expo-adapter';
// expo-file-system's native module, once the adapter installed Expo's globals (ADR 0028).
import './expo-file-system';
import { TurboModuleRegistry } from 'react-native';
import AssetSourceResolver from 'react-native/Libraries/Image/AssetSourceResolver';
import resolveAssetSource from 'react-native/Libraries/Image/resolveAssetSource';

const { frameworkURL } = TurboModuleRegistry.getEnforcing('OxyOSRuntime').getConstants();
// The directory the framework's assets are in, as React Native gives a bundle loaded from a file to
// its AssetSourceResolver: the URL up to its last '/'.
const frameworkDirectory = frameworkURL.slice(0, frameworkURL.lastIndexOf('/') + 1);

resolveAssetSource.addCustomSourceTransformer((resolver) =>
  resolver.asset.oxyosFramework
    ? new AssetSourceResolver(undefined, frameworkDirectory, resolver.asset).defaultAsset()
    : null,
);
