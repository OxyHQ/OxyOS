import './oxyos/expo-adapter';
import { AppRegistry } from 'react-native';
import App from './App';
import AndroidProbe from './AndroidProbe';

AppRegistry.registerComponent('OxyBloom', () => App);
AppRegistry.registerComponent('AndroidProbe', () => AndroidProbe);
