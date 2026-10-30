// A probe installed as an application of its own, asking for notifications (PermissionProbe.tsx).
import '../global.css';
import { AppRegistry } from 'react-native';
import PermissionProbe from '../PermissionProbe';

AppRegistry.registerComponent('PermissionProbe', () => PermissionProbe);
