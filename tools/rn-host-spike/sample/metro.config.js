// Metro for the OxyOS platform: React Native's JS, React and actual Bloom from the Bloom checkout.
const path = require('path');
const BLOOM = process.env.BLOOM_DIR;
const NODE_MODULES = require('fs').realpathSync(path.join(BLOOM, 'node_modules'));
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

const { withReactNativeCSS } = bloomRequire('react-native-css/metro');

module.exports = withReactNativeCSS(mergeConfig(getDefaultConfig(__dirname), {
  watchFolders: [BLOOM, NODE_MODULES, __dirname],
  resolver: {
    platforms: [PLATFORM, 'android', 'ios', 'native', 'web'],
    // The Bloom checkout's own worktrees are not part of the app.
    blockList: [new RegExp(path.join(BLOOM, '.worktrees').replace(/[/\\.]/g, '\\$&') + '.*')],
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
      let result;
      try {
        result = context.resolveRequest(context, moduleName, platform);
      } catch (error) {
        // A module that only exists as platform variants (.android/.ios/.web): OxyOS takes the
        // Android one, React Native's generic TurboModule-backed implementation.
        if (platform === PLATFORM && moduleName.startsWith('.')) {
          const base = path.resolve(path.dirname(context.originModulePath), moduleName);
          for (const ext of ['.android.tsx', '.android.ts', '.android.js', '.android.jsx']) {
            if (fs.existsSync(base + ext)) return { type: 'sourceFile', filePath: base + ext };
          }
        }
        throw error;
      }
      if (platform !== PLATFORM || result.type !== 'sourceFile') return result;
      const variant = oxyosVariant(result.filePath);
      if (variant) return { type: 'sourceFile', filePath: variant };
      return result;
    },
  },
  transformer: {
    // The registry `react-native`'s AssetRegistry export reads in 0.88 (expo-asset uses it too).
    assetRegistryPath: path.join(__dirname, 'oxyos/asset-registry.js'),
    babelTransformerPath: bloomRequire.resolve('@react-native/metro-babel-transformer'),
  },
  projectRoot: __dirname,
}), { disableTypeScriptGeneration: true });
