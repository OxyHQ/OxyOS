// Expo's Babel transformer (metro.config.js), with what babel-preset-expo's production build reads
// from its caller given as this platform's build means it:
//
// - The platform each file sees. The preset replaces `Platform.OS` and `Platform.select` with the
//   bundle's platform (`oxyos`); React Native's own modules run its Android implementation and read
//   'android' (metro.config.js, `oxyosVariant`), so theirs is 'android'.
// - An application's checkout as the project root when one is bundled (EXPO_PROJECT_ROOT,
//   oxyos/application-build.js), as Expo CLI gives it bundling that application: the preset
//   computes expo-router's `_ctx` root (`app/`) from it and refuses one outside it. Metro's own
//   project root is this sample's, which the file Metro names is relative to, so it is given
//   absolute.
const path = require('path');

const bloomRequire = require('module').createRequire(path.join(process.env.BLOOM_DIR, 'package.json'));
const expo = bloomRequire('@expo/metro-config/build/babel-transformer');
const RN_DIR = path.dirname(bloomRequire.resolve('react-native/package.json')) + path.sep;
const APP_DIR = process.env.EXPO_PROJECT_ROOT;

module.exports = {
  ...expo,
  getCacheKey: (options) => `${expo.getCacheKey(options)}\0${APP_DIR ?? ''}`,
  transform(args) {
    const filename = path.resolve(args.options.projectRoot, args.filename);
    const options = { ...args.options };
    if (filename.startsWith(RN_DIR)) options.platform = 'android';
    if (APP_DIR) {
      options.projectRoot = APP_DIR;
      options.customTransformOptions = { ...options.customTransformOptions, routerRoot: 'app' };
    }
    return expo.transform({ ...args, filename: APP_DIR ? filename : args.filename, options });
  },
};
