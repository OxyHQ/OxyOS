// An application that quits unexpectedly while its data is corrupt (CrashProbe.tsx), installed as
// an application of its own for the crash scenarios.
import '../global.css';
import { AppRegistry } from 'react-native';
import CrashProbe from '../CrashProbe';

AppRegistry.registerComponent('CrashProbe', () => CrashProbe);
