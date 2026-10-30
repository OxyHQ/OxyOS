// Bundles the app for the `oxyos` platform and copies its assets where React Native's
// AssetSourceResolver looks for them next to a file:// bundle. React Native runs its Android
// implementation on OxyOS (metro.config.js), so that is Android's resource layout:
// `<folder for the scale>/<resource identifier>.<type>`, computed by React Native's own helpers.
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';

const bloom = process.env.BLOOM_DIR;
// Expo's runtime replaces the global `fetch` with expo/fetch, whose native module OxyOS does not
// provide; this, Expo's own switch, keeps React Native's fetch, which OxyOS answers (Networking,
// crates/oxyos-net). It is inlined into the bundle (oxyos/inline-public-env.js).
process.env.EXPO_PUBLIC_USE_RN_FETCH = '1';
const require = createRequire(path.join(bloom, 'package.json'));
const Metro = require('metro');
const { getAndroidResourceFolderName, getAndroidResourceIdentifier } = require(
  '@react-native/assets-registry/path-support',
);
const [entry = 'index.js', outDir = 'build'] = process.argv.slice(2);

// The app's stylesheet imports `@oxy.so/bloom` and scans its source as an app's does from its
// node_modules (global.css): link the Bloom checkout there.
const linked = path.resolve('node_modules/@oxy.so/bloom');
fs.mkdirSync(path.dirname(linked), { recursive: true });
fs.rmSync(linked, { force: true });
fs.symlinkSync(bloom, linked);

// Transform output depends on config (asset registry, platform resolution): never reuse a cache.
const config = await Metro.loadConfig({ config: path.resolve('metro.config.js'), resetCache: true });
const out = path.resolve(outDir, 'index.js');
// With HERMESC (the hermesc of the Hermes the host runs), the bundle is Hermes bytecode, as an
// Android release build ships it: the Android Gradle plugin compiles Metro's output with
// `-emit-binary -O`. Without it, the bundle is source, which Hermes parses and compiles at startup.
const hermesc = process.env.HERMESC;
const source = hermesc ? path.resolve(outDir, 'index.source.js') : out;
fs.mkdirSync(path.dirname(out), { recursive: true });
const result = await Metro.runBuild(config, {
  entry, out: source, platform: 'oxyos', dev: false, minify: false, assets: true,
});
if (hermesc) {
  execFileSync(hermesc, ['-emit-binary', '-O', '-w', '-out', out, source], { stdio: 'inherit' });
  fs.rmSync(source);
}
// The application's public app config, embedded as an Android build embeds it (expo-constants'
// `getAppConfig` writes `app.config` into the APK's assets): next to the bundle, by the module that
// runs it, where the host's ExponentConstants reads it (oxyos/expo-adapter.js).
const application = require(path.resolve('oxyos/application-build.js'));
if (application && path.resolve(entry) === path.resolve('oxyos/stash-application.js')) {
  const config = path.resolve(outDir, 'applications', 'Stash', 'app.config');
  fs.mkdirSync(path.dirname(config), { recursive: true });
  fs.writeFileSync(config, JSON.stringify(application.config));
}
let copied = 0;
for (const asset of result.assets ?? []) {
  asset.files.forEach((file, index) => {
    const folder = getAndroidResourceFolderName(asset, asset.scales[index]);
    const dest = path.resolve(outDir, folder, `${getAndroidResourceIdentifier(asset)}.${asset.type}`);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(file, dest);
    copied += 1;
  });
}
// The runtime's framework (OXYOS_FRAMEWORK_BUILD, metro.config.js): what it provides, the manifest
// applications are built against: every module's key and id, whether an application may name it,
// and the version of each package (docs/adr/0030-installing-applications.mdx, decision 17).
if (process.env.OXYOS_FRAMEWORK_BUILD === '1') {
  const { recorded } = createRequire(import.meta.url)('./framework-ids.js');
  const modules = {};
  const packages = {};
  for (const described of recorded.values()) {
    if (described.app) continue;
    modules[described.key] = { id: described.id, public: described.public };
    if (described.name) packages[described.name] = described.version;
  }
  fs.writeFileSync(path.resolve(outDir, 'framework.json'), JSON.stringify({ packages, modules }));
}
console.log(JSON.stringify({ bundle: out, bytecode: Boolean(hermesc), assets: copied }));
