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
// - The React Compiler, when the application's config asks Expo for it (`experiments.reactCompiler`,
//   OXYOS_REACT_COMPILER from metro.config.js): the `reactCompiler` transform option Expo CLI gives
//   it, which the preset reads as its caller's `supportsReactCompiler` and answers with the
//   compiler and its own options. For the application's own sources alone: the preset would add it
//   to every file outside a `node_modules`, the platform's own modules among them, which the
//   application's own build takes from `node_modules` and never compiles.
const path = require('path');

const bloomRequire = require('module').createRequire(path.join(process.env.BLOOM_DIR, 'package.json'));
const expo = bloomRequire('@expo/metro-config/build/babel-transformer');
const RN_DIR = path.dirname(bloomRequire.resolve('react-native/package.json')) + path.sep;
const APP_DIR = process.env.EXPO_PROJECT_ROOT;
const REACT_COMPILER = process.env.OXYOS_REACT_COMPILER === '1';

module.exports = {
  ...expo,
  getCacheKey: (options) => `${expo.getCacheKey(options)}\0${APP_DIR ?? ''}\0${REACT_COMPILER}`,
  transform(args) {
    const filename = path.resolve(args.options.projectRoot, args.filename);
    const options = { ...args.options };
    if (filename.startsWith(RN_DIR)) options.platform = 'android';
    if (APP_DIR) {
      options.projectRoot = APP_DIR;
      options.customTransformOptions = { ...options.customTransformOptions, routerRoot: 'app' };
      if (REACT_COMPILER && filename.startsWith(APP_DIR + path.sep)) options.customTransformOptions.reactCompiler = true;
    }
    return expo.transform({ ...args, filename: APP_DIR ? filename : args.filename, options });
  },
};
