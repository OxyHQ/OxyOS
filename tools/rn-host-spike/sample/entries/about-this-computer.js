// About This Computer (apps/settings/src/AboutThisComputer.tsx): the Oxy menu's first row.
// Its package's bundle (docs/adr/0030-installing-applications.mdx, decision 1): this application
// alone, built on its own by packages.mjs against the runtime's framework.
import '../global.css';
import { AppRegistry } from 'react-native';
import AboutThisComputerApp from '../AboutThisComputerApp';

AppRegistry.registerComponent('OxyOSAboutThisComputer', () => AboutThisComputerApp);
