// The login screen (apps/login, docs/adr/0020-oxy-account-login.mdx).
// Its package's bundle (docs/adr/0030-installing-applications.mdx, decision 1): this application
// alone, built on its own by packages.mjs.
import '../oxyos/expo-adapter';
import '../global.css';
import { AppRegistry } from 'react-native';
import LoginSession from '../LoginSession';

AppRegistry.registerComponent('OxyOSLogin', () => LoginSession);
