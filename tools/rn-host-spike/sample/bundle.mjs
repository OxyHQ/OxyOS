// Bundles the app for the `oxyos` platform and copies its assets where React Native's
// AssetSourceResolver looks for them next to a file:// bundle.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';

const bloom = process.env.BLOOM_DIR;
const require = createRequire(path.join(bloom, 'package.json'));
const Metro = require('metro');
const [entry = 'index.js', outDir = 'build'] = process.argv.slice(2);

// Transform output depends on config (asset registry, platform resolution): never reuse a cache.
const config = await Metro.loadConfig({ config: path.resolve('metro.config.js'), resetCache: true });
const out = path.resolve(outDir, 'index.js');
fs.mkdirSync(path.dirname(out), { recursive: true });
const result = await Metro.runBuild(config, {
  entry, out, platform: 'oxyos', dev: false, minify: false, assets: true,
});
let copied = 0;
for (const asset of result.assets ?? []) {
  const base = asset.httpServerLocation.replace(/^\//, '').replace(/\.\.\//g, '_');
  asset.files.forEach((file, index) => {
    const scale = asset.scales[index];
    const suffix = scale === 1 ? '' : `@${scale}x`;
    const dest = path.resolve(outDir, base, `${asset.name}${suffix}.${asset.type}`);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(file, dest);
    copied += 1;
  });
}
console.log(JSON.stringify({ bundle: out, assets: copied }));
