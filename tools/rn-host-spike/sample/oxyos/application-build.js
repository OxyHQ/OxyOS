/**
 * An Expo Router application bundled from its own checkout, as the OxyOS runtime builds it
 * (metro.config.js, bundle.mjs): the checkout `OXYOS_APPLICATION_DIR` names (its Expo project,
 * as Stash's or Atlas's `packages/frontend`), as the module `OXYOS_APPLICATION_MODULE` (its
 * package's `entry.module`, the registry entry's `X-OxyOS-Bloom-Module`, beside which its app
 * config is kept). One build holds one
 * application; packages.mjs builds each application named by its own checkout's variable
 * (`STASH_DIR`, `ATLAS_DIR`) this way. Without one the bundle has no application and builds as
 * before.
 *
 * What the application's own Expo build would take from its project, read here the same way:
 * its environment files (`@expo/env`, as the Expo CLI loads them before bundling, so its
 * `EXPO_PUBLIC_*` variables are inlined), its public app config (`expo/config`'s `getConfig`, as
 * expo-constants' `getAppConfig` script embeds it in an Android build), its router root and its
 * dependencies. Everything is required from the application's own tree, never the platform's.
 *
 * Node-side only: nothing here is bundled.
 */
const fs = require('fs');
const path = require('path');
const { createRequire } = require('module');

const named = process.env.OXYOS_APPLICATION_DIR;
const module_ = process.env.OXYOS_APPLICATION_MODULE;

function fail(message) {
  throw new Error(`OXYOS_APPLICATION_DIR (${named}): ${message}`);
}

/** The git checkout that holds `dir`: where its installed dependencies are (a monorepo's root). */
function checkoutOf(dir) {
  for (let at = dir; ; at = path.dirname(at)) {
    if (fs.existsSync(path.join(at, '.git'))) return at;
    if (path.dirname(at) === at) fail('is not inside a git checkout');
  }
}

/**
 * Its TypeScript `paths` (`@/*`), which Expo's Metro resolves for the application's own sources
 * (`tsconfigPaths`), read by its own TypeScript (`extends` followed): each pattern's prefix and
 * suffix around its `*`, and the absolute targets to try in order.
 */
function pathAliases(dir, appRequire) {
  let ts;
  try {
    ts = appRequire('typescript');
  } catch {
    return [];
  }
  const file = ts.findConfigFile(dir, ts.sys.fileExists, 'tsconfig.json');
  if (!file || path.dirname(file) !== dir) return [];
  const read = ts.readConfigFile(file, ts.sys.readFile);
  if (read.error) fail(`its tsconfig.json cannot be read: ${ts.flattenDiagnosticMessageText(read.error.messageText, '\n')}`);
  const { options } = ts.parseJsonConfigFileContent(read.config, ts.sys, dir);
  const base = options.baseUrl ?? options.pathsBasePath ?? dir;
  return Object.entries(options.paths ?? {}).map(([pattern, targets]) => {
    const star = pattern.indexOf('*');
    return {
      prefix: star < 0 ? pattern : pattern.slice(0, star),
      suffix: star < 0 ? null : pattern.slice(star + 1),
      targets: targets.map((target) => path.resolve(base, target)),
    };
  });
}

/** What `alias` (one of pathAliases) makes of `name`: the paths to try, or null if it does not match. */
function aliased(alias, name) {
  if (alias.suffix === null) return name === alias.prefix ? alias.targets : null;
  if (!name.startsWith(alias.prefix) || !name.endsWith(alias.suffix) || name.length < alias.prefix.length + alias.suffix.length) return null;
  const star = name.slice(alias.prefix.length, name.length - alias.suffix.length);
  return alias.targets.map((target) => target.replace('*', star));
}

/**
 * Whether the application's own Oxy SDK has the store's installer seam (`@oxy.so/core` 7.10 and
 * later, `setStoreInstaller`), into which the runtime registers the machine's installer for an
 * application the policy gives `store` (oxyos/application.js). An application without it, or
 * without the SDK, is never given one.
 */
function hasStoreInstallerSeam(appRequire) {
  let manifest;
  try {
    manifest = appRequire('@oxy.so/core/package.json');
  } catch {
    return false;
  }
  const semver = createRequire(path.join(process.env.BLOOM_DIR, 'package.json'))('semver');
  return semver.gte(semver.coerce(manifest.version) ?? '0.0.0', '7.10.0');
}

function describe() {
  if (!named) return null;
  if (!module_ || !/^[A-Za-z][A-Za-z0-9]*$/.test(module_)) {
    fail(`needs OXYOS_APPLICATION_MODULE, the module it registers as (${JSON.stringify(module_)})`);
  }
  const dir = fs.realpathSync(path.resolve(named));
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8'));
  // The runtime runs the application's root exactly as `expo-router/entry` does; an application
  // with another entry is not one it knows how to start.
  if (manifest.main !== 'expo-router/entry') {
    fail(`is not an Expo Router application (its main is ${JSON.stringify(manifest.main)})`);
  }
  const appRequire = createRequire(path.join(dir, 'package.json'));
  // Its .env files for a production build (`.env.production.local`, `.env.local`,
  // `.env.production`, `.env`), before its config is read: app.config.js may read them too. As
  // Expo's loader, a variable already set wins. NODE_ENV is left as it is: the platform's build
  // decides how the bundle is compiled.
  const env = appRequire('@expo/env');
  env.loadEnvFiles(env.getFiles('production', { silent: true }).map((file) => path.join(dir, file)), { force: true });
  const { exp } = appRequire('expo/config').getConfig(dir, {
    isPublicConfig: true,
    skipSDKVersionRequirement: true,
  });
  // Expo Router's root: `app/`, the one babel-preset-expo's router plugin computes from the
  // project root (EXPO_PROJECT_ROOT, metro.config.js). Another root is refused rather than
  // half-supported.
  const configured = exp.extra?.router?.root;
  if ((configured && path.resolve(dir, configured) !== path.join(dir, 'app')) || fs.existsSync(path.join(dir, 'src/app'))) {
    fail('has a router root other than app/, which the OxyOS build does not support yet');
  }
  if (!fs.existsSync(path.join(dir, 'app'))) fail('has no app/ directory');
  const checkout = checkoutOf(dir);
  // The build the Oxy SDK busts its persisted caches on, as @oxy.so/app-preset names a production
  // build: the application's commit (oxyos/inline-public-env.js inlines it).
  process.env.OXY_BUILD_ID ??= require('child_process')
    .execFileSync('git', ['rev-parse', 'HEAD'], { cwd: dir, encoding: 'utf8' })
    .trim();
  const dependencies = { ...manifest.dependencies, ...manifest.devDependencies };
  return {
    dir,
    checkout,
    /** The module it registers as (oxyos/application.js), and where its app config is kept. */
    module: module_,
    appRoot: path.join(dir, 'app'),
    storeInstaller: hasStoreInstallerSeam(appRequire),
    // Its public app config, as `Constants.expoConfig` gives it (bundle.mjs writes it next to the
    // bundle, where the host's ExponentConstants reads it, as Android reads `app.config` from the
    // APK's assets).
    config: exp,
    // NativeWind (an Oxy app's, through @oxy.so/app-preset) gives every `react-native` import of
    // the application `className`: react-native-css's resolver with `globalClassNamePolyfill`.
    nativeWind: 'nativewind' in dependencies,
    /** The files its TypeScript `paths` make of a bare `name` imported by its own sources, in order. */
    aliases: (() => {
      const aliases = pathAliases(dir, appRequire);
      return (name) => aliases.map((alias) => aliased(alias, name)).find(Boolean) ?? null;
    })(),
    /** Whether `file` is the application's: its own sources or what its checkout installed. */
    owns(file) {
      return file === checkout || file.startsWith(checkout + path.sep);
    },
  };
}

module.exports = describe();
