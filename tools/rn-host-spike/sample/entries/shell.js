// The OxyOS shell (apps/shell), run as the session's shell (docs/adr/0009-session-shell.mdx).
// Its package's bundle (docs/adr/0030-installing-applications.mdx, decision 1): this application
// alone, built on its own by packages.mjs against the runtime's framework, which has React Native,
// React, Bloom and Expo's native side (framework.mjs).
import '../global.css';
import { AppRegistry } from 'react-native';
import { ShellSession } from '../ShellProbe';

AppRegistry.registerComponent('OxyOSShell', () => ShellSession);
