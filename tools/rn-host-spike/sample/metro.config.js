// Metro for the OxyOS platform: React Native's JS, React and actual Bloom from the Bloom checkout.
const path = require('path');
const BLOOM = process.env.BLOOM_DIR;
const NODE_MODULES = path.join(BLOOM, 'node_modules');
const bloomRequire = require('module').createRequire(path.join(BLOOM, 'package.json'));
const { getDefaultConfig, mergeConfig } = bloomRequire('@react-native/metro-config');

const fs = require('fs');
const PLATFORM = 'oxyos';
const RN_DIR = path.dirname(bloomRequire.resolve('react-native/package.json'));
const OVERRIDES = path.join(__dirname, 'oxyos');

// React Native files split by platform: an OxyOS override when one exists, otherwise the
// Android variant (React Native's generic TurboModule-backed implementation).
function oxyosVariant(filePath) {
  if (!filePath.startsWith(RN_DIR + path.sep) || !filePath.endsWith('.js')) return null;
  const relative = path.relative(RN_DIR, filePath).replace(/\.js$/, '');
  const override = path.join(OVERRIDES, relative + '.oxyos.js');
  if (fs.existsSync(override)) return override;
  const android = path.join(RN_DIR, relative + '.android.js');
  return fs.existsSync(android) ? android : null;
}

// `@oxy.so/bloom` always means the Bloom checkout (BLOOM_DIR), never a copy another package
// installed, resolved through its own `exports` map with the `react-native` condition.
const BLOOM_PACKAGE = bloomRequire(path.join(BLOOM, 'package.json'));
function bloomEntry(moduleName) {
  const subpath = '.' + moduleName.slice('@oxy.so/bloom'.length);
  const target = BLOOM_PACKAGE.exports[subpath === '.' ? '.' : subpath];
  const pick = (value) => (typeof value === 'string' ? value : pick(value['react-native'] ?? value.default));
  return target ? path.join(BLOOM, pick(target)) : null;
}

// Reanimated and Worklets have no OxyOS native module yet. Worklets uses its JS implementation
// (every non-native file, as on web): worklets run on the JS thread. Reanimated keeps its native
// files and only swaps its module for JSReanimated, the configuration Reanimated itself uses under
// Jest; animated props then update through Fabric's setNativeProps on the JS thread.
const WORKLETS = path.sep + 'react-native-worklets' + path.sep;
const REANIMATED_MODULE = /react-native-reanimated\/src\/ReanimatedModule\/(index|reanimatedModuleInstance)\.native\.ts$/;
function jsVariant(filePath) {
  if (!filePath.includes(WORKLETS) && !REANIMATED_MODULE.test(filePath)) return null;
  const match = filePath.match(/^(.*)\.native(\.[cm]?[jt]sx?)$/);
  if (!match) return null;
  const plain = match[1] + match[2];
  return fs.existsSync(plain) ? plain : null;
}

module.exports = mergeConfig(getDefaultConfig(__dirname), {
  watchFolders: [BLOOM, __dirname],
  resolver: {
    platforms: [PLATFORM, 'android', 'ios', 'native', 'web'],
    nodeModulesPaths: [NODE_MODULES],
    unstable_enablePackageExports: true,
    unstable_conditionNames: ['react-native', 'require', 'default'],
    resolveRequest(context, moduleName, platform) {
      if (moduleName === '@oxy.so/bloom' || moduleName.startsWith('@oxy.so/bloom/')) {
        const entry = bloomEntry(moduleName);
        if (!entry) throw new Error(`@oxy.so/bloom does not export ${moduleName}`);
        // Resolve without the extension so platform variants (.native.ts, ...) still apply.
        return context.resolveRequest(context, entry.replace(/\.[cm]?[jt]sx?$/, ''), platform);
      }
      const result = context.resolveRequest(context, moduleName, platform);
      if (platform !== PLATFORM || result.type !== 'sourceFile') return result;
      const variant = oxyosVariant(result.filePath);
      if (variant) return { type: 'sourceFile', filePath: variant };
      const js = jsVariant(result.filePath);
      return js ? { type: 'sourceFile', filePath: js } : result;
    },
  },
  transformer: {
    // React Native 0.88 keeps the asset registry in @react-native/assets-registry.
    assetRegistryPath: bloomRequire.resolve('@react-native/assets-registry/registry'),
    babelTransformerPath: bloomRequire.resolve('@react-native/metro-babel-transformer'),
  },
  projectRoot: __dirname,
});
