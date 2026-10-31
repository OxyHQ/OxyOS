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

// An Expo Router application bundled from its own checkout (OXYOS_APPLICATION_DIR,
// oxyos/application-build.js), registered as its module by oxyos/application.js; without one that
// module is empty. The runtime's modules that run inside the application (its entry, and the
// installer's registration in its own Oxy SDK, oxyos/store-registration.js) resolve what they name
// as the application's root would.
const APP = require('./oxyos/application-build');
const APP_ENTRY = path.join(__dirname, 'oxyos', 'application.js');
const STORE_REGISTRATION = path.join(__dirname, 'oxyos', 'store-registration.js');
const APP_ENTRIES = new Set([APP_ENTRY, STORE_REGISTRATION]);
if (APP) {
  // babel-preset-expo's router plugin replaces `process.env.EXPO_ROUTER_APP_ROOT` (expo-router's
  // `_ctx`) with the path from the file to the project's `app/`, the project root
  // oxyos/babel-transformer.js gives it from here, in the workers Metro forks.
  process.env.EXPO_PROJECT_ROOT = APP.dir;
  // Its React Compiler, when its config asks Expo for it (oxyos/babel-transformer.js).
  process.env.OXYOS_REACT_COMPILER = APP.config.experiments?.reactCompiler ? '1' : '';
}
const isBare = (name) => !name.startsWith('.') && !path.isAbsolute(name);
const packageOf = (name) => name.split('/').slice(0, name.startsWith('@') ? 2 : 1).join('/');
const BLOOM_ORIGIN = path.join(BLOOM, 'package.json');

// One copy of each package, shared by the platform and the application. Built against a runtime's
// framework (docs/adr/0030-installing-applications.mdx, decision 17), a package the framework has
// is exactly the framework's version (applicationRequest's `ruled`): an application that does not
// accept it stops the build. Otherwise a package the Bloom
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
const SAMPLE_MODULES = path.join(__dirname, 'node_modules') + path.sep;
const SAMPLE_ORIGIN = path.join(__dirname, 'package.json');
/**
 * The runtime's one copy of a package (docs/adr/0030-installing-applications.mdx, decision 17):
 * Bloom's tree and this folder's node_modules (the Expo modules the host implements) may each have
 * one; a request takes the newest of them its requester accepts, so every requester that can share
 * a copy shares the same one (Bloom's packages accept expo-constants ~58.0.12, the runtime's Expo
 * modules ask ~58.0.14: both take 58.0.14). None accepted, it resolves as it would.
 */
function sharedRequest(context, moduleName, platform) {
  if (!isBare(moduleName)) return context.resolveRequest(context, moduleName, platform);
  const name = packageOf(moduleName);
  const candidates = [];
  for (const origin of [BLOOM_ORIGIN, SAMPLE_ORIGIN]) {
    try {
      const found = context.resolveRequest({ ...context, originModulePath: origin }, moduleName, platform);
      if (found.type === 'sourceFile' && accepted(context.originModulePath, name, found.filePath)) candidates.push(found);
    } catch {}
  }
  const version = (found) => manifestOf(found.filePath)?.json.version ?? '0.0.0';
  candidates.sort((a, b) => semver.rcompare(version(a), version(b)));
  return candidates[0] ?? context.resolveRequest(context, moduleName, platform);
}
const inBloom = (file) => [BLOOM, NODE_MODULES].some((root) => file.startsWith(root + path.sep));
const APP_ORIGIN = APP && path.join(APP.dir, 'package.json');

// The application's requests (its sources, what its checkout installed, and its entry here).
function applicationRequest(context, moduleName, platform, resolve) {
  // The runtime's entry resolves what it starts (expo-router, the application's Oxy SDK) as the
  // application's root would.
  const entry = APP_ENTRIES.has(context.originModulePath);
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
    // A package the framework has (from the Bloom tree or this folder's: the Expo modules the host
    // implements), or one the Bloom tree has.
    if (isBare(name) && (FRAMEWORK?.packages[packageOf(name)] || fs.existsSync(path.join(NODE_MODULES, packageOf(name))))) {
      let shared = FRAMEWORK?.packages[packageOf(name)]
        ? sharedRequest({ ...c, resolveRequest: resolve }, name, p)
        : resolve({ ...c, originModulePath: BLOOM_ORIGIN }, name, p);
      // A package with an `import` and a `require` build of a module (react-native-css's
      // components) has one in the framework, the one the framework reached: an application
      // reaching it the other way gets that one rather than a second copy.
      if (FRAMEWORK?.packages[packageOf(name)] && shared.type === 'sourceFile' && !FRAMEWORK.modules[ids.keyOf(shared.filePath, ID_OPTIONS).key]) {
        const other = sharedRequest({ ...c, isESMImport: !c.isESMImport, resolveRequest: resolve }, name, p);
        if (other.type === 'sourceFile' && FRAMEWORK.modules[ids.keyOf(other.filePath, ID_OPTIONS).key]) shared = other;
      }
      // Built against a runtime's framework (docs/adr/0030-installing-applications.mdx, decision
      // 17), a package the framework has is the framework's, exactly its version: an application
      // that does not accept that version stops the build rather than bringing another copy.
      const provided = FRAMEWORK?.packages[packageOf(name)];
      if (provided && shared.type === 'sourceFile') {
        if (!accepted(c.originModulePath, packageOf(name), shared.filePath)) {
          throw new Error(`${c.originModulePath} does not accept ${packageOf(name)} ${provided.join(', ')}, the framework's (runtime ${FRAMEWORK.runtime.abi}, api ${FRAMEWORK.runtime.api})`);
        }
        return shared;
      }
      if (shared.type !== 'sourceFile' || accepted(c.originModulePath, packageOf(name), shared.filePath)) return shared;
    }
    return resolve(c, name, p);
  };
  // NativeWind, as the application's own build applies it: its `react-native` imports get
  // `className` (react-native-css's components, resolved by the same rule).
  return APP.nativeWind && !entry ? classNameResolver(ruled, origin, moduleName, platform) : ruled(origin, moduleName, platform);
}


// The runtime's framework and the applications built against it (docs/adr/0030-installing-
// applications.mdx, decision 17). Every build names modules by stable ids (framework-ids.js).
// OXYOS_FRAMEWORK_BUILD=1 builds the framework: React Native's InitializeCore and the runtime's
// start (oxyos/framework-init.js) run first, and its assets are marked as the framework's.
// OXYOS_FRAMEWORK=<framework.json> builds an application against that framework: its bundle has
// only its own modules, with no prelude, polyfills or InitializeCore (the framework's), and the
// build stops if it needs a module of a framework package the framework does not provide (another
// version, or a private module).
const ids = require('./framework-ids');
const ID_OPTIONS = { runtime: OVERRIDES, repository: path.resolve(__dirname, '../../..') };
const FRAMEWORK_BUILD = process.env.OXYOS_FRAMEWORK_BUILD === '1';
const FRAMEWORK = process.env.OXYOS_FRAMEWORK
  ? JSON.parse(fs.readFileSync(process.env.OXYOS_FRAMEWORK, 'utf8'))
  : null;
if (FRAMEWORK_BUILD && FRAMEWORK) throw new Error('a build is the framework or an application against one');

// The modules of a framework package an application builds for itself (docs/adr/0030-installing-
// applications.mdx, decision 17), because they hold the application: expo-router's context of its
// routes (`_ctx*`, a require.context of the application's app/) and the entry over it.
const PER_APPLICATION = [/[\\/]node_modules[\\/]expo-router[\\/]_ctx[^\\/]*$/,
  /[\\/]node_modules[\\/]expo-router[\\/]build[\\/]qualified-entry\.js$/];

function applicationModule(module) {
  // The statement running the application's entry (a script Metro appends, `require-<path>`) is the
  // application's; the prelude and the polyfills (scripts, not modules) are the framework's.
  if (module.path.startsWith('require-')) return true;
  if (module.output.some((output) => output.type.startsWith('js/script'))) return false;
  const described = ids.keyOf(module.path, ID_OPTIONS);
  // The framework's modules, its internals among them (an application reaches them through the
  // public ones it names), are the framework's.
  const framework = `the framework (runtime ${FRAMEWORK.runtime.abi}, api ${FRAMEWORK.runtime.api})`;
  if (PER_APPLICATION.some((pattern) => pattern.test(module.path))) return true;
  if (FRAMEWORK.modules[described.key]) {
    // The framework's, if it is the framework's version: an application resolving another copy
    // of a framework package (its own tree's) stops the build.
    const has = described.name && FRAMEWORK.packages[described.name];
    if (has && !has.includes(described.version)) {
      throw new Error(`${described.name} ${described.version} is not ${framework}'s, which has ${has.join(', ')} (${module.path})`);
    }
    return false;
  }
  if (described.name && FRAMEWORK.packages[described.name]) {
    const has = FRAMEWORK.packages[described.name];
    throw new Error(has.includes(described.version)
      ? `${framework} does not provide ${described.key} (${module.path})`
      : `${described.name} ${described.version} is not ${framework}'s, which has ${has.join(', ')} (${module.path})`);
  }
  // What the application names of the framework is public.
  for (const dependency of module.dependencies.values()) {
    const key = ids.keyOf(dependency.absolutePath, ID_OPTIONS).key;
    const named = FRAMEWORK.modules[key];
    if (named && !named.public) throw new Error(`${module.path} names ${key}, private to ${framework}`);
  }
  return true;
}

const serializer = {
  createModuleIdFactory: ids.createModuleIdFactory(ID_OPTIONS, ids.recorded),
  ...(FRAMEWORK_BUILD && {
    getModulesRunBeforeMainModule: () => [
      path.join(RN_DIR, 'Libraries', 'Core', 'InitializeCore.js'),
      path.join(OVERRIDES, 'framework-init.js'),
    ],
  }),
  ...(FRAMEWORK && {
    getModulesRunBeforeMainModule: () => [],
    getPolyfills: () => [],
    processModuleFilter: applicationModule,
  }),
};

module.exports = withReactNativeCSS(mergeConfig(getDefaultConfig(__dirname), {
  serializer,
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
        // The platform's own applications (apps/, this repository) declare the Bloom they need in
        // their package.json: a checkout of another stops the build. An application from its own
        // checkout (OXYOS_APPLICATION_DIR) is built as it is.
        if (!(APP && APP.owns(context.originModulePath)) &&
            !accepted(context.originModulePath, '@oxy.so/bloom', path.join(BLOOM, 'package.json'))) {
          const declared = manifestOf(context.originModulePath).json;
          throw new Error(`${declared.name} needs @oxy.so/bloom ${declared.dependencies['@oxy.so/bloom']}; ` +
            `BLOOM_DIR (${BLOOM}) is ${BLOOM_PACKAGE.version}`);
        }
        const entry = bloomEntry(moduleName);
        if (!entry) throw new Error(`@oxy.so/bloom does not export ${moduleName}`);
        // Resolve without the extension so platform variants (.native.ts, ...) still apply.
        return context.resolveRequest(context, entry.replace(/\.[cm]?[jt]sx?$/, ''), platform);
      }
      let result;
      try {
        if (APP && (APP_ENTRIES.has(context.originModulePath) || APP.owns(context.originModulePath))) {
          result = applicationRequest(context, moduleName, platform, context.resolveRequest);
        } else if (inBloom(context.originModulePath) || context.originModulePath.startsWith(SAMPLE_MODULES)) {
          // The runtime's own packages (this folder's node_modules: the Expo modules the host
          // implements) share the Bloom tree's copy of a package they accept, so the runtime and
          // its framework have one copy of each (one expo-constants, one expo-modules-core).
          result = sharedRequest(context, moduleName, platform);
        } else {
          result = context.resolveRequest(context, moduleName, platform);
        }
        // No application: its entry is an empty module, and the bundle has no application. An
        // application whose Oxy SDK has no store installer seam is given no installer.
        if (!APP && result.type === 'sourceFile' && result.filePath === APP_ENTRY) return { type: 'empty' };
        if (result.type === 'sourceFile' && result.filePath === STORE_REGISTRATION && !APP?.storeInstaller) {
          return { type: 'sourceFile', filePath: path.join(__dirname, 'oxyos', 'no-store-registration.js') };
        }
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
    // Expo's Babel transformer, as an Expo application's own build uses it: it tells
    // babel-preset-expo whether the build is a development one (the caller's `isDev`), so a
    // production build inlines `__DEV__` and `process.env.NODE_ENV` and the transform worker
    // (Expo's, through react-native-css) folds what is development-only away, React Native's
    // developer tools (react-devtools-core, LogBox) among it. React Native's own transformer gives
    // the preset no such caller, and every bundle kept them.
    babelTransformerPath: path.join(__dirname, 'oxyos', 'babel-transformer.js'),
    // expo-router's `_ctx`: a require.context of the application's router root.
    unstable_allowRequireContext: Boolean(APP),
    ...(FRAMEWORK_BUILD && { assetPlugins: [path.join(OVERRIDES, 'framework-asset-plugin.js')] }),
  },
  projectRoot: __dirname,
  // As @oxy.so/app-preset configures it: 1rem = 16px, variables resolved at runtime.
}), { disableTypeScriptGeneration: true, inlineRem: 16, inlineVariables: false });
