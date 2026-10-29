import './oxyos/expo-adapter';
import './global.css';
import { AppRegistry } from 'react-native';
import App from './App';
import AndroidProbe from './AndroidProbe';
import ClipboardProbe from './ClipboardProbe';
import EditingProbe from './EditingProbe';
import ImageProbe from './ImageProbe';
import SecureProbe from './SecureProbe';
import NetworkProbe from './NetworkProbe';

AppRegistry.registerComponent('OxyBloom', () => App);
AppRegistry.registerComponent('AndroidProbe', () => AndroidProbe);
AppRegistry.registerComponent('SecureProbe', () => SecureProbe);
AppRegistry.registerComponent('EditingProbe', () => EditingProbe);
AppRegistry.registerComponent('NetworkProbe', () => NetworkProbe);
AppRegistry.registerComponent('ImageProbe', () => ImageProbe);
AppRegistry.registerComponent('ClipboardProbe', () => ClipboardProbe);
