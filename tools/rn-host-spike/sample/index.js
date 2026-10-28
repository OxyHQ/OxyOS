import './oxyos/expo-adapter';
import './global.css';
import { AppRegistry } from 'react-native';
import App from './App';
import AndroidProbe from './AndroidProbe';
import EditingProbe from './EditingProbe';
import SecureProbe from './SecureProbe';

AppRegistry.registerComponent('OxyBloom', () => App);
AppRegistry.registerComponent('AndroidProbe', () => AndroidProbe);
AppRegistry.registerComponent('SecureProbe', () => SecureProbe);
AppRegistry.registerComponent('EditingProbe', () => EditingProbe);
