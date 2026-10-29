const path = require('path');
const bloomRequire = require('module').createRequire(path.join(process.env.BLOOM_DIR, 'package.json'));
// The preset real Oxy apps use (through @oxy.so/app-preset); it adds the worklets plugin itself.
// Expo's production build inlines EXPO_PUBLIC_ variables, which `expo` itself reads
// (oxyos/inline-public-env.js); plugins run before presets, so the preset never sees them.
module.exports = {
  plugins: [require.resolve('./oxyos/inline-public-env.js')],
  presets: [bloomRequire.resolve('babel-preset-expo')],
};
