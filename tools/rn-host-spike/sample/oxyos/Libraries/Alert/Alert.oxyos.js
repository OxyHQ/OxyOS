/**
 * React Native's Alert on OxyOS. Its Android implementation shows a native dialog through
 * DialogManagerAndroid and returns silently when that module is missing, as it is on OxyOS:
 * no dialog would appear and no button's callback would ever run. OxyOS has no native dialogs,
 * so Alert fails visibly instead (docs/react-native-android-adaptation.mdx). Bloom's own alert
 * surfaces (`@oxy.so/bloom`, with Alert's signature) draw their dialogs in the application.
 */
const UNAVAILABLE = 'is unavailable on OxyOS: it has no native dialogs (Bloom draws its own)';

class Alert {
  static alert() {
    throw new Error(`Alert.alert ${UNAVAILABLE}`);
  }

  static prompt() {
    throw new Error(`Alert.prompt ${UNAVAILABLE}`);
  }
}

export default Alert;
