/**
 * The OxyOS shell (apps/shell) in one window of the Bloom host, with a small fixture: a first look
 * at what the native host draws of it and what it lacks. Not the session's shell, which runs as
 * shell surfaces with the compositor's snapshot.
 */
import * as React from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { Shell } from '../../../apps/shell/src/Shell';
import type { ShellActions, ShellSnapshot } from '../../../apps/shell/src/model';

const icon = (name: string) => {
  switch (name) {
    case 'astro':
      return require('../../shell-preview/assets/apps/astro.png');
    case 'inbox':
      return require('../../shell-preview/assets/apps/inbox.png');
    case 'mention':
      return require('../../shell-preview/assets/apps/mention.png');
    default:
      return require('../../shell-preview/assets/apps/allo.png');
  }
};

// One vector icon, as the application registry may give one.
const ALIA =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#d269e6"/></svg>';

const snapshot: ShellSnapshot = {
  applications: [
    { id: 'astro', name: 'Astro', icon: icon('astro'), pinned: true },
    { id: 'inbox', name: 'Inbox', icon: icon('inbox'), pinned: true },
    { id: 'mention', name: 'Mention', icon: icon('mention'), pinned: true },
    { id: 'alia', name: 'Alia', icon: { svg: ALIA }, pinned: true },
    { id: 'allo', name: 'Allo', icon: icon('allo') },
  ],
  folders: [],
  windows: [
    {
      id: 'inbox-1',
      appId: 'inbox',
      title: 'Inbox',
      workspaceId: '1',
      thumbnail: require('../../shell-preview/assets/surfaces/inbox.png'),
      thumbnailSize: { width: 1600, height: 1066 },
    },
  ],
  workspaces: [
    { id: '1', name: 'Escritorio 1' },
    { id: '2', name: 'Escritorio 2' },
  ],
  workspaceId: '1',
  activeAppId: 'inbox',
  menus: [
    { label: 'OxyOS', items: [{ id: 'system.about', label: 'Acerca de OxyOS', enabled: false }] },
    {
      label: 'Ver',
      items: [
        { id: 'view.overview', label: 'Ver escritorios' },
        { id: 'view.launcher', label: 'Ver aplicaciones' },
      ],
    },
  ],
};

const actions: ShellActions = {
  activateApp: () => {},
  focusWindow: () => {},
  closeWindow: () => {},
  activateWorkspace: () => {},
  moveWindowToWorkspace: () => {},
  addWorkspace: () => {},
  menuCommand: () => {},
};

export default function ShellProbe() {
  return (
      <SafeAreaProvider>
        <BloomProvider mode="light" colorPreset="graphite" haptics={false} locale="es" platform={oxyosSurfaces}>
          <Shell
            snapshot={snapshot}
            actions={actions}
            wallpaper={require('../../../assets/wallpapers/solar-gradients/frames/frame-08.jpg')}
            now={new Date(2026, 9, 29, 9, 41)}
          />
        </BloomProvider>
      </SafeAreaProvider>
  );
}
