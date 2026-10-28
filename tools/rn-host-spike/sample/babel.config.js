const path = require('path');
const bloomRequire = require('module').createRequire(path.join(process.env.BLOOM_DIR, 'package.json'));
// The preset real Oxy apps use (through @oxy.so/app-preset); it adds the worklets plugin itself.
module.exports = { presets: [bloomRequire.resolve('babel-preset-expo')] };
