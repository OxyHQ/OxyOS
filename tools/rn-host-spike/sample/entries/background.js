// A probe installed as an application of its own, with a login item and background activity
// (BackgroundProbe.tsx, docs/adr/0060-login-items-and-background-activity.mdx).
import '../global.css';
import { AppRegistry } from 'react-native';
import BackgroundProbe from '../BackgroundProbe';

AppRegistry.registerComponent('BackgroundProbe', () => BackgroundProbe);
