// Builds a runtime's framework (docs/adr/0030-installing-applications.mdx, decision 17): React
// Native's JavaScript, React, Bloom and the packages the host's native side implements, compiled
// once into the runtime, which every application's JavaScript runtime evaluates before its own
// bundle.
//
//   node framework.mjs RUNTIME [PREVIOUS]
//
// with BLOOM_DIR (the Bloom checkout, whose node_modules are the packages built from). RUNTIME is a
// runtime's directory (runtime.json, hermesc); this writes `framework/` there: `framework.hbc`, its
// assets, and `framework.json`, what it provides (every module's key and id, which are public, and
// each package's version), and names its digest in runtime.json.
//
// PREVIOUS, the framework.json of the runtime's previous `api` of the same `abi`, is the contract
// this one keeps: an `api` only adds (decision 2), so every public module it had must be here with
// the same id; a framework that drops or changes one needs a new `abi`, and this refuses it.
//
// What it provides is its entry, generated here: each public module below as a getter, as React
// Native's own index.js exposes its modules, so that every one is in the framework and none runs
// until an application requires it. The entry (`generated/framework.js`) is build output, derived
// at every build from BLOOM_DIR's exports, and is not tracked (`.gitignore`): building against
// another Bloom never changes the tree. What a framework provides is recorded where it is kept,
// in its `framework.json`, and what it must keep is checked against PREVIOUS above.
import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const [runtimeDir, previousManifest] = process.argv.slice(2);
const bloom = process.env.BLOOM_DIR;
if (!runtimeDir || !bloom) throw new Error('usage: BLOOM_DIR=... node framework.mjs RUNTIME');
const runtime = path.resolve(runtimeDir);
const level = JSON.parse(fs.readFileSync(path.join(runtime, 'runtime.json'), 'utf8'));

// React Native and React, which must be the host's native side's exactly, and the helpers Babel's
// output for any application calls (which ones depends on the syntax it uses: every one).
const nodeModules = fs.realpathSync(path.join(bloom, 'node_modules'));
const babelHelpers = fs.readdirSync(path.join(nodeModules, '@babel', 'runtime', 'helpers'))
  .filter((file) => file.endsWith('.js'))
  .sort()
  .map((file) => `@babel/runtime/helpers/${file.slice(0, -3)}`);
const core = ['react', 'react/jsx-runtime', 'react/compiler-runtime', 'react-native', ...babelHelpers];
// The packages whose native side the host implements: their JavaScript is the runtime's too.
const native = [
  'react-native-reanimated', 'react-native-worklets', 'react-native-gesture-handler', 'react-native-svg',
  'react-native-svg/css', 'react-native-safe-area-context', 'react-native-screens',
  'expo', 'expo-modules-core', 'expo-asset', 'expo-font', 'expo-blur', 'expo-haptics', 'expo-image',
  'expo-constants', 'expo-linking', 'expo-keep-awake', 'expo-glass-effect',
  'expo-file-system', 'expo-file-system/legacy', 'expo-document-picker',
  // What the host gives Expo applications (docs/react-native-android-adaptation.mdx): AsyncStorage,
  // NetInfo, expo-crypto, expo-secure-store, expo-web-browser, the clipboard, notifications and the
  // application's facts.
  '@react-native-async-storage/async-storage', '@react-native-community/netinfo', 'expo-crypto',
  'expo-secure-store', 'expo-web-browser', 'expo-clipboard', 'expo-notifications', 'expo-application',
  // What an Expo Router application's entry runs on (expo-router's qualified entry, but its context
  // of the application's routes, `_ctx`, which each application builds for itself; decision 17).
  'expo-router', 'expo-router/build/ExpoRoot', 'expo-router/build/head', 'expo-router/build/fast-refresh',
  '@expo/metro-runtime',
  // Worklets' manifest, which a package built on worklets reads for the version it runs on
  // (react-native-audio-api's check, in Atlas through the Alia SDK).
  'react-native-worklets/package.json',
];
// Bloom (and what its stylesheet compiles to at run time), every export an application may import
// but those that are not code an application runs (its Tailwind preset, its CSS and token files,
// and its Expo config plugin, `app.plugin.js`, which Expo's prebuild runs under Node and which
// needs Node's modules, as `assert`).
const bloomManifest = JSON.parse(fs.readFileSync(path.join(bloom, 'package.json'), 'utf8'));
// The oldest Bloom the runtime's own parts work with: 15.91.0, whose ContextMenu hangs from a view
// it has no trigger around (`anchorRef`), as the text fields' context menu does
// (oxyos/text-menu.js, docs/adr/0037-text-editing.mdx). Older, the menu would never open.
const BLOOM_AT_LEAST = [15, 91, 0];
const bloomVersion = bloomManifest.version.split(/[.-]/).slice(0, 3).map(Number);
const older = BLOOM_AT_LEAST.findIndex((part, index) => bloomVersion[index] !== part);
if (older >= 0 && bloomVersion[older] < BLOOM_AT_LEAST[older]) {
  throw new Error(`Bloom ${bloomManifest.version} in ${bloom} is older than ${BLOOM_AT_LEAST.join('.')}, which the runtime needs`);
}
const notCode = new Set(['./package.json', './tailwind-preset', './design-tokens/theme.css', './design-tokens/tokens.json',
  './app.plugin.js']);
const bloomExports = [];
for (const subpath of Object.keys(bloomManifest.exports)) {
  if (notCode.has(subpath)) continue;
  const specifier = subpath === '.' ? '@oxy.so/bloom' : `@oxy.so/bloom/${subpath.slice(2)}`;
  if (!subpath.includes('*')) {
    bloomExports.push(specifier);
    continue;
  }
  // A pattern (`./icons/Ri*`): every file its target matches.
  const target = bloomManifest.exports[subpath]['react-native'].default;
  const [prefix, suffix] = target.split('*');
  const directory = path.join(bloom, path.dirname(prefix));
  for (const file of fs.readdirSync(directory).sort()) {
    const relative = `./${path.relative(bloom, path.join(directory, file))}`;
    if (relative.startsWith(prefix) && relative.endsWith(suffix)) {
      const match = relative.slice(prefix.length, relative.length - suffix.length);
      bloomExports.push(specifier.replace('*', match));
    }
  }
}
const css = [
  'nativewind',
  'react-native-css', 'react-native-css/native', 'react-native-css/native-internal', 'react-native-css/components',
  'react-native-css/components/react-native-safe-area-context', 'react-native-css/utilities',
];
// The runtime's own start, Bloom's platform adapter for OxyOS, and the store's installer the runtime
// registers in an application the policy gives `store` (oxyos/store-installer.js).
const own = ['./oxyos/framework-init', './oxyos/platform-surface', './oxyos/store-installer',
  'react-native/Libraries/Core/InitializeCore'];

const provided = [...core, ...native, ...css, ...bloomExports, ...own];
const generated = path.join(here, 'generated');
fs.mkdirSync(generated, { recursive: true });
const entry = path.join(generated, 'framework.js');
fs.writeFileSync(entry, [
  '// Generated by framework.mjs: what the runtime\'s framework provides, each a getter.',
  'module.exports = {',
  ...provided.map((specifier) => {
    const request = specifier.startsWith('./') ? `.${specifier}` : specifier;
    return `  get ${JSON.stringify(specifier)}() { return require(${JSON.stringify(request)}); },`;
  }),
  '};',
  '',
].join('\n'));

// Built beside the runtime's framework, and put in its place only once it is whole and keeps its
// contract.
const out = path.join(runtime, 'framework.new');
fs.rmSync(out, { recursive: true, force: true });
// A build that fails leaves nothing beside the runtime's framework.
process.on('exit', (code) => {
  if (code !== 0) fs.rmSync(out, { recursive: true, force: true });
});
// No application (OXYOS_APPLICATION_DIR): the framework is the runtime's, built from no
// application's checkout.
const { OXYOS_APPLICATION_DIR: _application, OXYOS_APPLICATION_MODULE: _module, ...environment } = process.env;
const built = execFileSync('node', ['bundle.mjs', path.relative(here, entry), out], {
  cwd: here,
  env: { ...environment, OXYOS_FRAMEWORK_BUILD: '1', HERMESC: '' },
  stdio: ['ignore', 'pipe', 'inherit'],
}).toString().trim().split('\n').pop();
const source = path.join(out, 'index.js');
const bytecode = path.join(out, 'framework.hbc');
execFileSync(path.join(runtime, 'hermesc'), ['-emit-binary', '-O', '-w', '-out', bytecode, source], { stdio: 'inherit' });
fs.rmSync(source);

const manifest = JSON.parse(fs.readFileSync(path.join(out, 'framework.json'), 'utf8'));
if (previousManifest) {
  const previous = JSON.parse(fs.readFileSync(previousManifest, 'utf8'));
  if (previous.runtime.abi !== level.abi || previous.runtime.api >= level.api) {
    throw new Error(`${previousManifest} is not an earlier api of runtime ${level.abi}`);
  }
  const lost = Object.entries(previous.modules)
    .filter(([key, module]) => module.public && manifest.modules[key]?.id !== module.id)
    .map(([key]) => key);
  if (lost.length > 0) {
    throw new Error(`api ${level.api} does not keep api ${previous.runtime.api}'s public modules (a new abi does): ${lost.slice(0, 20).join(', ')}${lost.length > 20 ? ', ...' : ''}`);
  }
}
fs.writeFileSync(path.join(out, 'framework.json'), JSON.stringify({ runtime: { abi: level.abi, api: level.api }, ...manifest }));
// The keyboard's focus ring in each of Bloom's accents (docs/adr/0039-keyboard-access.mdx): the host
// draws it, as a Mac draws its ring in the accent colour, in the `ring` token of the session's
// accent, light or dark, as Bloom's own design tokens resolve it (W3C design tokens,
// color.<preset>.<scheme>.ring): Bloom's colours, never a copy of them.
const tokens = JSON.parse(fs.readFileSync(path.join(bloom, 'src', 'design-tokens', 'tokens.json'), 'utf8'));
const rings = Object.fromEntries(Object.entries(tokens.color)
  .filter(([preset, schemes]) => !preset.startsWith('$') && schemes.light?.ring && schemes.dark?.ring)
  .map(([preset, schemes]) => [preset, { light: schemes.light.ring.$value, dark: schemes.dark.ring.$value }]));
if (Object.keys(rings).length === 0) throw new Error('Bloom\'s design tokens name no focus ring');
fs.writeFileSync(path.join(out, 'focus-rings.json'), JSON.stringify(rings));
fs.rmSync(path.join(runtime, 'framework'), { recursive: true, force: true });
fs.renameSync(out, path.join(runtime, 'framework'));
const bytes = fs.readFileSync(path.join(runtime, 'framework', 'framework.hbc'));
const digest = crypto.createHash('sha256').update(bytes).digest('hex');
const manifestDigest = crypto.createHash('sha256')
  .update(fs.readFileSync(path.join(runtime, 'framework', 'framework.json'))).digest('hex');
fs.writeFileSync(path.join(runtime, 'runtime.json'), JSON.stringify({
  abi: level.abi,
  api: level.api,
  framework: { sha256: digest, size: bytes.length, manifest: manifestDigest },
}) + '\n');
const count = Object.keys(manifest.modules).length;
console.log(JSON.stringify({ framework: path.join(runtime, 'framework', 'framework.hbc'), bytes: bytes.length, modules: count, built: JSON.parse(built) }));
