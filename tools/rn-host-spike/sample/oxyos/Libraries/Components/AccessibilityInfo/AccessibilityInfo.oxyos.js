/**
 * React Native's AccessibilityInfo on OxyOS: its Android implementation, which the host's
 * `AccessibilityInfo` module answers (docs/react-native-android-adaptation.mdx), with reduce
 * transparency answered as iOS answers it. Android has no such setting, so React Native's Android
 * AccessibilityInfo says it is off without asking and never tells a change; OxyOS has it
 * (docs/adr/0019-settings.mdx), so `isReduceTransparencyEnabled` asks the session's, and
 * `reduceTransparencyChanged` follows it as the host tells it.
 *
 * metro.config.js resolves this file wherever React Native's is named; its own import below is
 * React Native's.
 */
import AccessibilityInfo from 'react-native/Libraries/Components/AccessibilityInfo/AccessibilityInfo';
import NativeAccessibilityInfo from 'react-native/Libraries/Components/AccessibilityInfo/NativeAccessibilityInfo';
import RCTDeviceEventEmitter from 'react-native/Libraries/EventEmitter/RCTDeviceEventEmitter';

const REDUCE_TRANSPARENCY_CHANGED = 'reduceTransparencyChanged';

const OxyOSAccessibilityInfo = {
  ...AccessibilityInfo,

  /** Whether the session's Reduce Transparency is on. */
  isReduceTransparencyEnabled() {
    return new Promise((resolve, reject) => {
      if (NativeAccessibilityInfo?.isReduceTransparencyEnabled != null) {
        NativeAccessibilityInfo.isReduceTransparencyEnabled(resolve);
      } else {
        reject(new Error('NativeAccessibilityInfoAndroid.isReduceTransparencyEnabled is not available'));
      }
    });
  },

  /** React Native's, and `reduceTransparencyChanged` as the session changes it. */
  addEventListener(eventName, handler) {
    if (eventName === REDUCE_TRANSPARENCY_CHANGED) {
      return RCTDeviceEventEmitter.addListener(REDUCE_TRANSPARENCY_CHANGED, handler);
    }
    return AccessibilityInfo.addEventListener(eventName, handler);
  },
};

export default OxyOSAccessibilityInfo;
