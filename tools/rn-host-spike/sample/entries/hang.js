// An application that hangs (HangProbe.tsx), installed as an application of its own for the hang
// scenarios (docs/adr/0067-hangs-and-force-quit.mdx).
import '../global.css';
import { AppRegistry } from 'react-native';
import HangProbe from '../HangProbe';

AppRegistry.registerComponent('HangProbe', () => HangProbe);
