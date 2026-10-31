// The fidelity gallery (sample/FidelityGallery.tsx) built for the web as an Oxy application ships
// Bloom to a browser: `expo export --platform web` (Atlas's `export:web`), Metro on Expo's config,
// react-native-web for React Native, react-native-css with its global `className` polyfill
// (NativeWind's `withNativewind`, as @oxy.so/app-preset applies it) and Tailwind compiled from the
// same stylesheet the host's bundle uses (sample/global.css). Bloom resolves from BLOOM_DIR's
// sources, its `.web` files taking the place Bloom's published `browser` build gives them.
//
// The fonts are the host's own files: Bloom's web font sheet names its `.woff2` files, and here each
// resolves to the `.ttf` beside it, the file the host registers (Bloom's FontLoader.native.tsx,
// TextEngine::registerFamily), so a difference between the two is drawing, never another font.
//
//   BLOOM_DIR=... node fidelity/web.mjs OUT
//
// writes OUT/project (the Expo project, generated: nothing here is a checkout of its own) and
// OUT/dist (the export: index.html, its script and stylesheet, the fonts).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const sample = path.resolve(here, '../sample');
const bloom = process.env.BLOOM_DIR && path.resolve(process.env.BLOOM_DIR);
if (!bloom) throw new Error('BLOOM_DIR: the Bloom checkout the web build uses (the host bundle uses the same)');
const [outArg] = process.argv.slice(2);
if (!outArg) throw new Error('usage: node fidelity/web.mjs OUT');
const out = path.resolve(outArg);
const project = path.join(out, 'project');
const dist = path.join(out, 'dist');
const nodeModules = fs.realpathSync(path.join(bloom, 'node_modules'));
// Tailwind's PostCSS plugin is the sample's (sample/package.json), as the host's bundle compiles it.
// Expo loads a plugin by `require` from the project, so its CommonJS entry by its file.
const tailwindPostcss = path.join(sample, 'node_modules', '@tailwindcss', 'postcss', 'dist', 'index.js');
if (!fs.existsSync(tailwindPostcss)) throw new Error(`${tailwindPostcss} is missing: run \`npm ci\` in ${sample}`);

fs.rmSync(project, { recursive: true, force: true });
fs.mkdirSync(path.join(project, 'public'), { recursive: true });
fs.symlinkSync(nodeModules, path.join(project, 'node_modules'));
const write = (name, text) => fs.writeFileSync(path.join(project, name), text);
const bloomVersion = JSON.parse(fs.readFileSync(path.join(bloom, 'package.json'), 'utf8')).version;
const expoVersion = JSON.parse(fs.readFileSync(path.join(nodeModules, 'expo', 'package.json'), 'utf8')).version;
write('package.json', JSON.stringify({
  name: 'oxyos-fidelity-web', private: true, main: 'index.js',
  dependencies: { expo: expoVersion, '@oxy.so/bloom': bloomVersion },
}, null, 2));
write('app.json', JSON.stringify({
  expo: { name: 'Bloom fidelity', slug: 'bloom-fidelity', platforms: ['web'], web: { bundler: 'metro', output: 'single' } },
}, null, 2));
write('babel.config.js', "module.exports = { presets: ['babel-preset-expo'] };\n");
write('postcss.config.mjs', `export default { plugins: { ${JSON.stringify(tailwindPostcss)}: {} } };\n`);
// An Oxy application's stylesheet as @oxy.so/app-preset's base.css makes it (Tailwind's layers,
// NativeWind's theme, whose `web:` variant Bloom's web layouts use, Bloom's tokens and the sans
// family restated after NativeWind's), scanning what the host bundle's (sample/global.css) scans.
write('global.css', [
  '@import "tailwindcss/theme.css" layer(theme);',
  '@import "tailwindcss/preflight.css" layer(base);',
  '@import "tailwindcss/utilities.css";',
  '@import "nativewind/theme";',
  `@import ${JSON.stringify(path.join(bloom, 'src/design-tokens/theme.css'))};`,
  ':root { --font-sans: var(--bloom-font-sans, Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif); }',
  `@source ${JSON.stringify(path.join(sample, '*.tsx'))};`,
  `@source ${JSON.stringify(path.join(sample, '../../../apps/shell/src/**/*.{ts,tsx}'))};`,
  `@source ${JSON.stringify(path.join(bloom, 'src/**/*.{ts,tsx}'))};`,
  '',
].join('\n'));
// The page: the window's size and nothing around it (no margin, no scroll bars).
write('public/index.html', `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Bloom fidelity</title>
    <style id="expo-reset">
      html, body { height: 100%; margin: 0; overflow: hidden; }
      ::-webkit-scrollbar { display: none; }
      #root { display: flex; height: 100%; flex: 1; }
    </style>
  </head>
  <body>
    <div id="root"></div>
  </body>
</html>
`);
write('index.js', `import './global.css';
import { AppRegistry } from 'react-native';
import FidelityGallery from ${JSON.stringify(path.join(sample, 'FidelityGallery'))};
import inter from ${JSON.stringify(path.join(bloom, 'src/fonts/assets/InterVariable.ttf'))};

// Text without a fontFamily is set in the platform's own face: react-native-web's System stack in
// a browser, Inter on OxyOS (the host's text engine). Each family of that stack is the file the
// host draws such text with, so the gallery compares drawing, not which faces a machine has.
const system = ['-apple-system', 'BlinkMacSystemFont', 'Segoe UI', 'Roboto', 'Helvetica', 'Arial'];
const faces = document.createElement('style');
faces.textContent = system
  .map((name) => \`@font-face { font-family: '\${name}'; src: url("\${inter}"); font-weight: 100 900; font-style: normal; }\`)
  .join('\\n');
document.head.appendChild(faces);

// The page from the address (?page=N), as the host's module name chooses it (FidelityGalleryN).
const page = Number(new URLSearchParams(window.location.search).get('page') || '1');
AppRegistry.registerComponent('FidelityGallery', () => FidelityGallery);
AppRegistry.runApplication('FidelityGallery', { rootTag: document.getElementById('root'), initialProps: { page } });
`);
write('metro.config.js', `// Generated by fidelity/web.mjs: Expo's Metro config as @oxy.so/app-preset builds it for the web.
const path = require('path');
const fs = require('fs');
const { getDefaultConfig } = require('expo/metro-config');
const { withReactNativeCSS } = require('react-native-css/metro');
const BLOOM = ${JSON.stringify(bloom)};
const NODE_MODULES = ${JSON.stringify(nodeModules)};
const SAMPLE = ${JSON.stringify(sample)};
const FONTS = path.join(BLOOM, 'src', 'fonts', 'assets') + path.sep;
const bloomPackage = JSON.parse(fs.readFileSync(path.join(BLOOM, 'package.json'), 'utf8'));
// '@oxy.so/bloom[/x]' is BLOOM_DIR's source (the condition the host's Metro picks), without its
// extension, so the platform's variants (.web.ts) apply as Bloom's own web build compiles them.
function bloomEntry(moduleName) {
  const subpath = '.' + moduleName.slice('@oxy.so/bloom'.length);
  const pick = (value) => (typeof value === 'string' ? value : pick(value['react-native'] ?? value.default));
  const exact = bloomPackage.exports[subpath];
  if (exact) return path.join(BLOOM, pick(exact));
  let best = null;
  for (const key of Object.keys(bloomPackage.exports)) {
    const star = key.indexOf('*');
    if (star < 0) continue;
    const [prefix, suffix] = [key.slice(0, star), key.slice(star + 1)];
    if (!subpath.startsWith(prefix) || !subpath.endsWith(suffix)) continue;
    if (!best || prefix.length > best.prefix.length) {
      best = { prefix, match: subpath.slice(prefix.length, subpath.length - suffix.length), target: bloomPackage.exports[key] };
    }
  }
  return best ? path.join(BLOOM, pick(best.target).replace('*', best.match)) : null;
}
const config = getDefaultConfig(__dirname);
config.watchFolders = [BLOOM, NODE_MODULES, SAMPLE];
config.resolver.nodeModulesPaths = [NODE_MODULES];
config.resolver.unstable_enablePackageExports = true;
config.resolver.assetExts = [...new Set([...config.resolver.assetExts, 'woff2', 'woff', 'ttf', 'wasm'])];
config.resolver.blockList = [path.join(BLOOM, '.worktrees'), path.join(BLOOM, '.claude')].map(
  (dir) => new RegExp(dir.replace(/[/\\\\.]/g, '\\\\$&') + '/.*'));
const parent = config.resolver.resolveRequest;
config.resolver.resolveRequest = (context, moduleName, platform) => {
  const resolve = parent ?? context.resolveRequest;
  if (moduleName === '@oxy.so/bloom' || moduleName.startsWith('@oxy.so/bloom/')) {
    const entry = bloomEntry(moduleName);
    if (!entry) throw new Error('@oxy.so/bloom does not export ' + moduleName);
    return context.resolveRequest(context, entry.replace(/\\.[cm]?[jt]sx?$/, ''), platform);
  }
  // Bloom's web fonts: the host's file, the .ttf beside each .woff2.
  if (moduleName.endsWith('.woff2') && context.originModulePath.startsWith(path.join(BLOOM, 'src', 'fonts'))) {
    const ttf = path.resolve(path.dirname(context.originModulePath), moduleName).replace(/\\.woff2$/, '.ttf');
    if (ttf.startsWith(FONTS) && fs.existsSync(ttf)) return context.resolveRequest(context, ttf, platform);
  }
  return resolve(context, moduleName, platform);
};
module.exports = withReactNativeCSS(config, {
  globalClassNamePolyfill: true, disableTypeScriptGeneration: true, inlineRem: 16, inlineVariables: false,
});
`);

fs.rmSync(dist, { recursive: true, force: true });
const cli = path.join(nodeModules, 'expo', 'bin', 'cli');
execFileSync(process.execPath, [cli, 'export', '--platform', 'web', '--output-dir', dist, '--clear'], {
  cwd: project,
  stdio: 'inherit',
  env: { ...process.env, CI: '1', EXPO_NO_TELEMETRY: '1', EXPO_OFFLINE: '1', NODE_ENV: 'production', EXPO_NO_DOTENV: '1' },
});
console.log(JSON.stringify({ dist }));
