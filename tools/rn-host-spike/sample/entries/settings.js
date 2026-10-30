// Settings (apps/settings, docs/adr/0019-settings.mdx).
// Its package's bundle (docs/adr/0030-installing-applications.mdx, decision 1): this application
// alone, built on its own by packages.mjs.
import '../oxyos/expo-adapter';
import '../global.css';
import { AppRegistry } from 'react-native';
import SettingsApp from '../SettingsApp';

AppRegistry.registerComponent('OxyOSSettings', () => SettingsApp);
