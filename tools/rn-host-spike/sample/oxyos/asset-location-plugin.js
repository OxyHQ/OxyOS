// Metro's asset plugin for every build: an asset from outside this repository (Bloom, its packages,
// an application's checkout) is named by where it is in that checkout, not by the way from this
// repository to it. Metro names an asset by its path from the project root (this sample), and
// Android's resource name (bundle.mjs, `getAndroidResourceIdentifier`) is that path flattened, so a
// package's names depended on where the checkouts sat: from a CI runner's directory ten levels up,
// Dropsea's icon font's name outgrew a package path's 128 characters (oxyos-package). The bundle
// registers the same location (Metro applies its plugins to both), so the runtime finds the file.
const fs = require('fs');
const path = require('path');

const REPOSITORY = path.resolve(__dirname, '../../../..');
const APP = require('./application-build');
const BLOOM = process.env.BLOOM_DIR;
// Each root by a name of its own; the most specific first (Bloom's packages are inside Bloom).
const ROOTS = [
  ...(APP ? [['app', APP.checkout]] : []),
  ['bloom-modules', BLOOM && path.join(BLOOM, 'node_modules')],
  ['bloom', BLOOM],
].filter(([, root]) => root).map(([name, root]) => [name, fs.realpathSync(root)]);

const inside = (root, file) => {
  const relative = path.relative(root, file);
  return relative && !relative.startsWith('..') && !path.isAbsolute(relative) ? relative : null;
};

module.exports = (asset) => {
  const directory = fs.realpathSync(asset.fileSystemLocation);
  if (inside(REPOSITORY, directory) !== null || directory === REPOSITORY) return asset;
  for (const [name, root] of ROOTS) {
    const relative = inside(root, directory);
    if (relative !== null) {
      return { ...asset, httpServerLocation: `/assets/${name}/${relative.split(path.sep).join('/')}` };
    }
  }
  return asset;
};
