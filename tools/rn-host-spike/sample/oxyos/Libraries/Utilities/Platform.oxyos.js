/**
 * React Native's Platform module for OxyOS. Constants come from the host's PlatformConstants
 * TurboModule (same spec React Native uses on Android). `Platform.select` prefers `oxyos`,
 * then `native`, then `default` — never another platform's key.
 */
import NativePlatformConstantsAndroid from 'react-native/Libraries/Utilities/NativePlatformConstantsAndroid';

const Platform = {
  __constants: null,
  OS: 'oxyos',
  get Version() {
    return this.constants.Version;
  },
  get constants() {
    if (this.__constants == null) {
      this.__constants = NativePlatformConstantsAndroid.getConstants();
    }
    return this.__constants;
  },
  get isTesting() {
    return __DEV__ ? this.constants.isTesting : false;
  },
  get isDisableAnimations() {
    return this.constants.isDisableAnimations ?? this.isTesting;
  },
  get isTV() {
    return false;
  },
  get isVision() {
    return false;
  },
  select: (spec) => ('oxyos' in spec ? spec.oxyos : 'native' in spec ? spec.native : spec.default),
};

export default Platform;
