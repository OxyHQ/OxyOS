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

const PLATFORM_MODULE = path.join('Libraries', 'Utilities', 'Platform');
// Third-party packages whose Android modules OxyOS runs, as it runs React Native's own.
const ANDROID_PACKAGES = ['expo-blur'];
const RN_ENTRY = path.join(RN_DIR, 'index.js');

// React Native files split by platform: an OxyOS override when one exists, otherwise the
// Android variant (React Native's generic TurboModule-backed implementation).
//
// React Native's own modules run its Android implementation, because the host implements React
// Native's Android interfaces (TurboModule specs, component names, commands); they read
// `Platform.OS` as 'android' and take their Android branches. Everything else, including what
// `react-native` exports to apps and Bloom, sees 'oxyos'.
function oxyosVariant(filePath, originModulePath) {
  if (!filePath.startsWith(RN_DIR + path.sep) || !filePath.endsWith('.js')) return null;
  const relative = path.relative(RN_DIR, filePath).replace(/\.js$/, '');
  const internal = originModulePath.startsWith(RN_DIR + path.sep) && originModulePath !== RN_ENTRY;
  if (relative === PLATFORM_MODULE && internal) return path.join(RN_DIR, relative + '.android.js');
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
  const pick = (value) => (typeof value === 'string' ? value : pick(value['react-native'] ?? value.default));
  const exact = BLOOM_PACKAGE.exports[subpath];
  if (exact) return path.join(BLOOM, pick(exact));
  // A subpath pattern (`./icons/Ri*`), as Node resolves one: the key with the longest prefix
  // before its `*` that matches, the matched part put in place of the target's `*`.
  let best = null;
  for (const key of Object.keys(BLOOM_PACKAGE.exports)) {
    const star = key.indexOf('*');
    if (star < 0) continue;
    const [prefix, suffix] = [key.slice(0, star), key.slice(star + 1)];
    if (!subpath.startsWith(prefix) || !subpath.endsWith(suffix) || subpath.length < key.length - 1) continue;
    if (!best || prefix.length > best.prefix.length) {
      best = { prefix, match: subpath.slice(prefix.length, subpath.length - suffix.length), target: BLOOM_PACKAGE.exports[key] };
    }
  }
  return best ? path.join(BLOOM, pick(best.target).replace('*', best.match)) : null;
}

const { withReactNativeCSS } = bloomRequire('react-native-css/metro');
// NativeWind's `className` resolution (`globalClassNamePolyfill`), which its package does not export.
const { nativeResolver: classNameResolver } = require(path.join(NODE_MODULES, 'react-native-css/dist/commonjs/metro/resolver.js'));

// An Expo Router application bundled from its own checkout (STASH_DIR, oxyos/application-build.js),
// registered as module `Stash` by oxyos/stash-application.js; without one that module is empty.
const APP = require('./oxyos/application-build');
const APP_ENTRY = path.join(__dirname, 'oxyos', 'stash-application.js');
if (APP) {
  // babel-preset-expo's router plugin replaces `process.env.EXPO_ROUTER_APP_ROOT` (expo-router's
  // `_ctx`) with the path from the file to the project's `app/`. React Native's transformer gives
  // Babel no project root, so the preset takes it from here, in the workers Metro forks.
  process.env.EXPO_PROJECT_ROOT = APP.dir;
}
const isBare = (name) => !name.startsWith('.') && !path.isAbsolute(name);
const packageOf = (name) => name.split('/').slice(0, name.startsWith('@') ? 2 : 1).join('/');
const BLOOM_ORIGIN = path.join(BLOOM, 'package.json');

// One copy of each package, shared by the platform and the application. A package the Bloom
// checkout's tree has resolves from there (the platform's runtime, matching the host's native
// code: one React, one React Native, one Reanimated...) when that copy is a version its requester
// accepts (the range its package declares); Bloom itself always from BLOOM_DIR (above); everything
// else, and what the Bloom tree has only in a version its requester does not accept, from the
// application's own node_modules. A Bloom checkout's tree is its development install: it can hold
// a package another of its packages does not accept (expo-router 58 asks for expo-linking 58, and
// Bloom's tree has 56), which then resolves from the application, never as the wrong version.
const semver = bloomRequire('semver');
const manifests = new Map();
/** The package `file` belongs to: its nearest package.json with a name, and where it is. */
function manifestOf(file) {
  const dir = path.dirname(file);
  if (!manifests.has(dir)) {
    let found = null;
    const own = path.join(dir, 'package.json');
    if (fs.existsSync(own)) {
      const json = JSON.parse(fs.readFileSync(own, 'utf8'));
      if (json.name) found = { dir, json };
    }
    manifests.set(dir, found ?? (path.dirname(dir) === dir ? null : manifestOf(dir)));
  }
  return manifests.get(dir);
}
/** Whether the package at `file` is a version the package of `requester` accepts for `name`. */
function accepted(requester, name, file) {
  const declared = manifestOf(requester)?.json;
  const range = declared?.dependencies?.[name] ?? declared?.peerDependencies?.[name] ?? declared?.optionalDependencies?.[name];
  // Undeclared, or not a version range (a workspace or file link, an alias): nothing to check.
  if (!range || !semver.validRange(range)) return true;
  const version = manifestOf(file)?.json.version;
  return !version || semver.satisfies(version, range, { includePrerelease: true });
}
const inBloom = (file) => [BLOOM, NODE_MODULES].some((root) => file.startsWith(root + path.sep));
const APP_ORIGIN = APP && path.join(APP.dir, 'package.json');

// The application's requests (its sources, what its checkout installed, and its entry here).
function applicationRequest(context, moduleName, platform, resolve) {
  // The runtime's entry resolves what it starts (expo-router) as the application's root would.
  const entry = context.originModulePath === APP_ENTRY;
  const origin = entry && isBare(moduleName) ? { ...context, originModulePath: APP_ORIGIN } : context;
  // Its own sources' TypeScript paths (`@/lib/config`), as Expo's Metro resolves them.
  const own = !entry && !context.originModulePath.includes(`${path.sep}node_modules${path.sep}`);
  const candidates = own && isBare(moduleName) ? APP.aliases(moduleName) : null;
  if (candidates) {
    for (const candidate of candidates) {
      try {
        return context.resolveRequest(context, candidate, platform);
      } catch {}
    }
    throw new Error(`${moduleName} (imported by ${context.originModulePath}) matches the application's TypeScript paths, but none of ${candidates.join(', ')} exists`);
  }
  const ruled = (c, name, p) => {
    if (isBare(name) && fs.existsSync(path.join(NODE_MODULES, packageOf(name)))) {
      const shared = resolve({ ...c, originModulePath: BLOOM_ORIGIN }, name, p);
      if (shared.type !== 'sourceFile' || accepted(c.originModulePath, packageOf(name), shared.filePath)) return shared;
    }
    return resolve(c, name, p);
  };
  // NativeWind, as the application's own build applies it: its `react-native` imports get
  // `className` (react-native-css's components, resolved by the same rule).
  return APP.nativeWind && !entry ? classNameResolver(ruled, origin, moduleName, platform) : ruled(origin, moduleName, platform);
}

// A request from the Bloom tree, with an application in the bundle: what its tree resolves,
// unless that is a version the requester does not accept and the application has one it does.
function platformRequest(context, moduleName, platform) {
  const result = context.resolveRequest(context, moduleName, platform);
  if (!isBare(moduleName) || result.type !== 'sourceFile') return result;
  const name = packageOf(moduleName);
  if (accepted(context.originModulePath, name, result.filePath)) return result;
  let theirs;
  try {
    theirs = context.resolveRequest({ ...context, originModulePath: APP_ORIGIN }, moduleName, platform);
  } catch {
    return result;
  }
  return theirs.type === 'sourceFile' && accepted(context.originModulePath, name, theirs.filePath) ? theirs : result;
}

module.exports = withReactNativeCSS(mergeConfig(getDefaultConfig(__dirname), {
  // The repository's own images (wallpapers, the shell's app artwork) for the image probe.
  watchFolders: [BLOOM, NODE_MODULES, __dirname, ...(APP ? [APP.checkout] : []), path.resolve(__dirname, '../../../assets'),
    path.resolve(__dirname, '../../shell-preview/assets'),
    // The shell's own sources (apps/shell), for the shell probe.
    path.resolve(__dirname, '../../../apps')],
  resolver: {
    platforms: [PLATFORM, 'android', 'ios', 'native', 'web'],
    // The Bloom checkout's own worktrees are not part of the app, nor the application checkout's.
    blockList: [BLOOM, ...(APP ? [APP.checkout] : [])].flatMap((root) => ['.worktrees', '.claude'].map((dir) =>
      new RegExp(path.join(root, dir).replace(/[/\\.]/g, '\\$&') + '/.*'))),
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
        if (APP && (context.originModulePath === APP_ENTRY || APP.owns(context.originModulePath))) {
          result = applicationRequest(context, moduleName, platform, context.resolveRequest);
        } else if (APP && inBloom(context.originModulePath)) {
          result = platformRequest(context, moduleName, platform);
        } else {
          result = context.resolveRequest(context, moduleName, platform);
        }
        // No application: its entry is an empty module, and the bundle has no Stash.
        if (!APP && result.type === 'sourceFile' && result.filePath === APP_ENTRY) return { type: 'empty' };
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
      const variant = oxyosVariant(result.filePath, context.originModulePath);
      if (variant) return { type: 'sourceFile', filePath: variant };
      // Packages whose Android native views the host implements as Android does take their
      // Android modules too: expo-blur's blur target is then its own view, never a View that
      // Fabric may flatten away (expo-blur.h).
      if (ANDROID_PACKAGES.some((name) => result.filePath.includes(`${path.sep}node_modules${path.sep}${name}${path.sep}`))) {
        const android = result.filePath.replace(/\.js$/, '.android.js');
        if (android !== result.filePath && fs.existsSync(android)) return { type: 'sourceFile', filePath: android };
      }
      return result;
    },
  },
  transformer: {
    // The registry `react-native`'s AssetRegistry export reads in 0.88 (expo-asset uses it too).
    assetRegistryPath: path.join(__dirname, 'oxyos/asset-registry.js'),
    babelTransformerPath: bloomRequire.resolve('@react-native/metro-babel-transformer'),
    // expo-router's `_ctx`: a require.context of the application's router root.
    unstable_allowRequireContext: Boolean(APP),
  },
  projectRoot: __dirname,
  // As @oxy.so/app-preset configures it: 1rem = 16px, variables resolved at runtime.
}), { disableTypeScriptGeneration: true, inlineRem: 16, inlineVariables: false });
